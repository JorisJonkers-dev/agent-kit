import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import type { BuildTrackerIssuesPort, BuildTrackerRepositoryPort, IssueRecord, RepositoryName } from '../contexts/build-tracker/index.js'
import { defaultBuildTrackerRuntime, runBuildTrackerCli, type BuildTrackerCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

const WIDGETS: RepositoryName = { host: 'github.com', owner: 'acme', repo: 'widgets' }

interface FakeIssue {
  readonly title?: string
  readonly state?: 'open' | 'closed'
  readonly subIssues?: readonly (number | string)[]
  /** Blockers: a number is an issue of acme/widgets, a string `owner/repo#n` (or `host/owner/repo#n`) any other. */
  readonly blockedBy?: readonly (number | string)[]
}

/** An issue tracker held in memory: issues keyed by `#n` for acme/widgets, or by `owner/repo#n`. */
class FakeTracker implements BuildTrackerIssuesPort {
  readonly calls: string[] = []
  failure: string | undefined
  /** Only calls starting with this fail; empty fails every call. */
  failOn = ''

  constructor(public issues: Record<string, FakeIssue>) {}

  issue(repository: RepositoryName, number: number): Promise<IssueRecord> {
    return this.answer(`issue ${key(repository, number)}`, () => this.record(key(repository, number)))
  }

  subIssues(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]> {
    return this.answer(`sub-issues ${key(repository, number)}`, () =>
      (this.find(key(repository, number)).subIssues ?? []).map((sub) => this.record(name(sub))),
    )
  }

  blockedBy(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]> {
    return this.answer(`blocked-by ${key(repository, number)}`, () =>
      (this.find(key(repository, number)).blockedBy ?? []).map((blocker) => this.record(name(blocker))),
    )
  }

  private answer<T>(call: string, read: () => T): Promise<T> {
    this.calls.push(call)
    const fails = this.failure !== undefined && call.startsWith(this.failOn)
    return fails ? Promise.reject(new Error(this.failure)) : Promise.resolve(read())
  }

  private find(ref: string): FakeIssue {
    const issue = this.issues[short(ref)]
    if (issue === undefined) throw new Error(`no issue ${ref}`)
    return issue
  }

  private record(ref: string): IssueRecord {
    const match = /^(?:([^/]+)\/)?([^/]+)\/([^/#]+)#(\d+)$/.exec(ref)
    const issue = this.issues[short(ref)] ?? {}
    return {
      number: Number(match?.[4]),
      repository: { host: match?.[1] ?? 'github.com', owner: String(match?.[2]), repo: String(match?.[3]) },
      state: issue.state ?? 'open',
      title: issue.title ?? '',
    }
  }
}

function key(repository: RepositoryName, number: number): string {
  return `${repository.host}/${repository.owner}/${repository.repo}#${String(number)}`
}

/** The fake's key for an issue: `#n` in acme/widgets, `owner/repo#n` elsewhere on github.com, the full reference on any other host. */
function short(ref: string): string {
  return ref.replace(/^github\.com\//, '').replace(/^acme\/widgets#/, '#')
}

function name(ref: number | string): string {
  return typeof ref === 'number' ? `github.com/acme/widgets#${String(ref)}` : ref
}

/** Every git repository the fake knows: its root and the remote it reports. */
class FakeRepositories implements BuildTrackerRepositoryPort {
  readonly roots: { root: string; remote: RepositoryName | undefined }[] = []

  locate(dir: string): ReturnType<BuildTrackerRepositoryPort['locate']> {
    const found = this.roots.find((entry) => dir === entry.root || dir.startsWith(`${entry.root}/`))
    if (found === undefined) return Promise.resolve(undefined)
    const rel = relative(found.root, dir)
    return Promise.resolve({ mainRoot: found.root, prefix: rel === '' ? '' : `${rel}/`, remote: found.remote, root: found.root })
  }
}

interface Seeded {
  /** The state home every state goes under. */
  readonly home: string
  /** The repository's root, also the working directory. */
  readonly root: string
  readonly tracker: FakeTracker
  readonly repositories: FakeRepositories
  run(...argv: string[]): Promise<BuildTrackerCliResult>
  /** The state file of acme/widgets#10, parsed. */
  state(path?: string): Promise<Record<string, unknown>>
}

/** Spec acme/widgets#10 with four tickets: #1 closed, #2 and #3 after it, #4 the live check after #3. */
function widgetIssues(): Record<string, FakeIssue> {
  return {
    '#10': { subIssues: [1, 2, 3, 4], title: 'Spec: widgets' },
    '#1': { state: 'closed', title: 'widget store' },
    '#2': { blockedBy: [1], title: 'widget list' },
    '#3': { blockedBy: [1, 2], title: 'widget search' },
    '#4': { blockedBy: [3], title: 'live check on widgets' },
  }
}

async function seeded(issues: Record<string, FakeIssue> = widgetIssues()): Promise<Seeded> {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'build-tracker-seed-')))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, 'state')
  const root = join(dir, 'widgets')
  await mkdir(root)
  const tracker = new FakeTracker(issues)
  const repositories = new FakeRepositories()
  repositories.roots.push({ remote: WIDGETS, root })
  const runtime = defaultBuildTrackerRuntime({
    clock: { now: () => new Date('2026-11-03T12:00:00Z') },
    cwd: root,
    env: { XDG_STATE_HOME: home },
    issues: tracker,
    repositories,
  })
  const statePath = join(home, 'agent-kit', 'build-tracker', 'github.com', 'acme', 'widgets', '10', 'state.json')
  return {
    home,
    repositories,
    root,
    run: (...argv) => runBuildTrackerCli(argv, runtime),
    state: async (path = statePath) => JSON.parse(await readFile(path, 'utf8')) as Record<string, unknown>,
    tracker,
  }
}

/** The Mermaid source as Mermaid reads it: the graph block's text, its HTML entities decoded. */
function graphSource(page: string): string {
  const block = /<pre class="mermaid">([\s\S]*?)<\/pre>/.exec(page)?.[1] ?? ''
  return block.replaceAll('&quot;', '"').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&')
}

const WIDGETS_DIR = ['agent-kit', 'build-tracker', 'github.com', 'acme', 'widgets', '10']

describe('build-tracker init on a GitHub spec', () => {
  it('writes one entry per sub-issue with its title, its blockers, and merged for a closed issue or todo for an open one', async () => {
    const s = await seeded()

    const result = await s.run('init', '--spec', 'acme/widgets#10')

    expect(result.stderr).toBe('')
    expect(result.exitCode).toBe(0)
    const dir = join(s.home, ...WIDGETS_DIR)
    expect(JSON.parse(result.stdout)).toEqual({
      milestones: [],
      page: join(dir, 'page.html'),
      spec: 'acme/widgets#10',
      state: join(dir, 'state.json'),
      status: 'initialized',
      tickets: 4,
    })
    expect(await s.state()).toEqual({
      spec: 'acme/widgets#10',
      tickets: {
        '1': { blocked_by: [], status: 'merged', title: 'widget store' },
        '2': { blocked_by: [1], status: 'todo', title: 'widget list' },
        '3': { blocked_by: [1, 2], status: 'todo', title: 'widget search' },
        '4': { blocked_by: [3], status: 'todo', title: 'live check on widgets' },
      },
      title: 'Spec: widgets',
      version: 1,
    })
  })
})

describe('build-tracker init spec references', () => {
  it.each(['#10', '10', 'https://github.com/acme/widgets/issues/10', 'github.com/acme/widgets#10'])(
    'reads %s as issue 10 of acme/widgets',
    async (ref) => {
      const s = await seeded()

      const result = await s.run('init', '--spec', ref)

      expect(result.exitCode).toBe(0)
      expect(JSON.parse(result.stdout)).toMatchObject({ spec: 'acme/widgets#10', status: 'initialized' })
      expect((await s.state()).spec).toBe('acme/widgets#10')
    },
  )

  it('keeps the host of a spec on another GitHub host, in the state and its path', async () => {
    const s = await seeded({
      'ghe.example.com/acme/widgets#7': { subIssues: ['ghe.example.com/acme/widgets#8'], title: 'Spec: enterprise' },
      'ghe.example.com/acme/widgets#8': { title: 'one ticket' },
    })

    const result = await s.run('init', '--spec', 'https://GHE.example.com/acme/widgets/issues/7')

    const output = JSON.parse(result.stdout) as Record<string, string>
    expect(output).toMatchObject({
      spec: 'ghe.example.com/acme/widgets#7',
      state: join(s.home, 'agent-kit', 'build-tracker', 'ghe.example.com', 'acme', 'widgets', '7', 'state.json'),
    })
    const page = join(s.home, 'p.html')
    await s.run('render', '--state', String(output.state), '--out', page)
    expect(await readFile(page, 'utf8')).toContain('<a href="https://ghe.example.com/acme/widgets/issues/8">#8</a>')
  })

  it('refuses a bare issue number outside a repository with a remote', async () => {
    const s = await seeded()
    s.repositories.roots[0] = { remote: undefined, root: s.root }

    const result = await s.run('init', '--spec', '#10')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'invalid-spec',
      message: `#10 names an issue of the current repository, but ${s.root} is in no repository with a remote; name it as owner/repo#<number>`,
      status: 'error',
    })
  })

  it('reports an issue tracker it cannot read, naming the spec', async () => {
    const s = await seeded()
    s.tracker.failure = 'gh api repos/acme/widgets/issues/10 exited 1: HTTP 404\nmore'

    const result = await s.run('init', '--spec', 'acme/widgets#10')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'tracker-unreachable',
      message: 'cannot read the tickets of acme/widgets#10: gh api repos/acme/widgets/issues/10 exited 1: HTTP 404',
      status: 'error',
    })
  })

  it.each([
    ['a ticket blockers', 'blocked-by', 'rate limited'],
    ['a failure with no message', 'sub-issues', ''],
  ])('reports an issue tracker that fails reading %s, and writes no state', async (_name, failOn, failure) => {
    const s = await seeded()
    s.tracker.failure = failure
    s.tracker.failOn = failOn

    const result = await s.run('init', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toEqual({
      code: 'tracker-unreachable',
      message: `cannot read the tickets of acme/widgets#10: ${failure === '' ? 'the tracker failed' : failure}`,
      status: 'error',
    })
    await expect(s.state()).rejects.toThrow(/ENOENT/)
  })

  it('refuses a spec with a sub-issue in another repository', async () => {
    const s = await seeded({ '#10': { subIssues: [1, 'acme/gadgets#3'], title: 'Spec' }, '#1': { title: 'one' } })

    const result = await s.run('init', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toEqual({
      code: 'unsupported-ticket',
      message:
        "acme/gadgets#3 is a sub-issue of acme/widgets#10 in another repository; the build tracker keys tickets by issue number in the spec's repository",
      status: 'error',
    })
  })
})

describe('build-tracker init milestones', () => {
  it('records each --milestone in the order given', async () => {
    const s = await seeded()

    const result = await s.run('init', '--spec', '#10', '--milestone', 'Phase A = #3', '--milestone', 'Live=4')

    expect(JSON.parse(result.stdout)).toMatchObject({ milestones: ['Phase A', 'Live'] })
    expect((await s.state()).milestones).toEqual([
      { closes: 3, name: 'Phase A' },
      { closes: 4, name: 'Live' },
    ])
  })

  it('keeps an = inside a milestone name', async () => {
    const s = await seeded()

    await s.run('init', '--spec', '#10', '--milestone', 'a=b=2')

    expect((await s.state()).milestones).toEqual([{ closes: 2, name: 'a=b' }])
  })

  it.each([
    ['no ticket', 'Phase A', 'invalid-milestone', 'Phase A is not <name>=<ticket>'],
    ['no name', '=3', 'invalid-milestone', '=3 is not <name>=<ticket>'],
    ['a ticket outside the spec', 'Phase A=9', 'unknown-ticket', "milestone Phase A closes ticket 9, which is not one of the spec's tickets"],
  ])('refuses a milestone with %s and writes no state', async (_name, milestone, code, message) => {
    const s = await seeded()

    const result = await s.run('init', '--spec', '#10', '--milestone', milestone)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code, message, status: 'error' })
    await expect(s.state()).rejects.toThrow(/ENOENT/)
  })
})

describe('build-tracker init on an existing state', () => {
  it('refuses, names sync, and leaves the state as it was', async () => {
    const s = await seeded()
    await s.run('init', '--spec', '#10')
    await s.run('set', '--state', join(s.home, ...WIDGETS_DIR, 'state.json'), '2=running')
    const before = await s.state()

    const result = await s.run('init', '--spec', '#10')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'state-exists',
      message: `a state for acme/widgets#10 already exists at ${join(s.home, ...WIDGETS_DIR, 'state.json')}; bring it up to date with sync`,
      status: 'error',
    })
    expect(await s.state()).toEqual(before)
  })

  it('refuses a state path it cannot read, saying to delete it and run init again', async () => {
    const s = await seeded()
    await mkdir(join(s.home, ...WIDGETS_DIR, 'state.json'), { recursive: true })

    const result = await s.run('init', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state', status: 'error' })
    expect(result.stderr).toMatch(/EISDIR.*; delete it and run init again\n$/)
  })

  it('refuses a state file it cannot read as a state with the way out, not with sync', async () => {
    const s = await seeded()
    const state = join(s.home, ...WIDGETS_DIR, 'state.json')
    await mkdir(join(s.home, ...WIDGETS_DIR), { recursive: true })
    await writeFile(state, '{ not json')

    const result = await s.run('init', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state', status: 'error' })
    expect(result.stderr).toMatch(new RegExp(`^build-tracker init: the state file at ${state} is unreadable: it is not JSON \\(.*\\); delete it and run init again\\n$`))
    expect(await readFile(state, 'utf8')).toBe('{ not json')
  })
})

describe('build-tracker sync', () => {
  const statePath = (s: Seeded): string => join(s.home, ...WIDGETS_DIR, 'state.json')

  it('leaves the state matching the issue tracker, with every status, note and key the agent set on an open ticket', async () => {
    const s = await seeded()
    await s.run('init', '--spec', '#10', '--milestone', 'Live=4')
    await s.run('set', '--state', statePath(s), '2=review:merging now', '3=owner:needs a key', '4=running')
    const edited = (await s.state()) as { tickets: Record<string, object> }
    edited.tickets['4'] = { ...edited.tickets['4'], live: true }
    await writeFile(statePath(s), JSON.stringify({ ...edited, subtitle: 'branch feat/widgets' }))
    s.tracker.issues = {
      ...s.tracker.issues,
      '#10': { subIssues: [1, 2, 3, 4, 5], title: 'Spec: widgets, renamed' },
      '#2': { blockedBy: [1], state: 'closed', title: 'widget list' },
      '#3': { blockedBy: [2], title: 'widget search, renamed' },
      '#5': { blockedBy: [4], title: 'widget docs' },
    }

    const result = await s.run('sync', '--spec', '#10')

    expect(result.stderr).toBe('')
    expect(JSON.parse(result.stdout)).toEqual({
      added: [5],
      merged: [2],
      page: join(s.home, ...WIDGETS_DIR, 'page.html'),
      spec: 'acme/widgets#10',
      state: statePath(s),
      status: 'synced',
      tickets: 5,
    })
    expect(await s.state()).toEqual({
      milestones: [{ closes: 4, name: 'Live' }],
      spec: 'acme/widgets#10',
      subtitle: 'branch feat/widgets',
      tickets: {
        '1': { blocked_by: [], status: 'merged', title: 'widget store' },
        '2': { blocked_by: [1], note: 'merging now', status: 'merged', title: 'widget list' },
        '3': { blocked_by: [2], note: 'needs a key', status: 'owner', title: 'widget search, renamed' },
        '4': { blocked_by: [3], live: true, status: 'running', title: 'live check on widgets' },
        '5': { blocked_by: [4], status: 'todo', title: 'widget docs' },
      },
      title: 'Spec: widgets',
      version: 1,
    })
  })

  it('keeps a ticket the issue tracker no longer lists', async () => {
    const s = await seeded()
    await s.run('init', '--spec', '#10')
    s.tracker.issues = { ...s.tracker.issues, '#10': { subIssues: [1, 2], title: 'Spec' } }

    const result = await s.run('sync', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toMatchObject({ added: [], merged: [], tickets: 4 })
    expect(Object.keys((await s.state()).tickets as object)).toEqual(['1', '2', '3', '4'])
  })

  it('on a spec with no state exits non-zero with an error that names init', async () => {
    const s = await seeded()

    const result = await s.run('sync', '--spec', '#10')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'missing-state',
      message: `there is no state file at ${statePath(s)}; create it with init`,
      status: 'error',
    })
    expect(s.tracker.calls).toEqual([])
  })
})

describe('build-tracker on a local markdown spec', () => {
  const SPEC = 'docs/specs/widgets/spec.md'
  const LOCAL_DIR = ['agent-kit', 'build-tracker', 'github.com', 'acme', 'widgets', 'docs-specs-widgets-spec-md-7f62ea6e']

  /** A spec beside an issues/ folder: 01 done, 02 after 01, 03 after 01 and 02, 04 after 03 and an issue of another spec. */
  async function localSpec(s: Seeded, extra: Record<string, string> = {}): Promise<void> {
    const dir = join(s.root, 'docs', 'specs', 'widgets')
    await mkdir(join(dir, 'issues'), { recursive: true })
    await writeFile(join(dir, 'spec.md'), '# Widgets spec\n\nThe widgets.\n')
    const tickets: Record<string, string> = {
      '01-store.md': '# 01: Widget store\n\nStatus: done\n\n## Blocked by\n\nNone (can start immediately)\n',
      '02-list.md': '# 02 - Widget list\n\n**Blocked by:** 01\n',
      '03-search.md': '# Widget search\n\n## Blocked by\n\n- 01-store.md\n- Widget list\n',
      '04-live.md': '# 04. Live check\n\nBlocked by: #3, acme/gadgets#5, 9\n',
      'notes.txt': 'not a ticket',
      ...extra,
    }
    await Promise.all(Object.entries(tickets).map(([name, text]) => writeFile(join(dir, 'issues', name), text)))
  }

  const localState = (s: Seeded): string => join(s.home, ...LOCAL_DIR, 'state.json')

  it('reads the numbered tickets in the issues/ folder beside the spec and their Blocked by lines', async () => {
    const s = await seeded()
    await localSpec(s)

    const result = await s.run('init', '--spec', SPEC)

    expect(result.stderr).toBe('')
    expect(JSON.parse(result.stdout)).toEqual({
      milestones: [],
      page: join(s.home, ...LOCAL_DIR, 'page.html'),
      spec: SPEC,
      state: localState(s),
      status: 'initialized',
      tickets: 4,
    })
    const issues = 'docs/specs/widgets/issues'
    expect(await s.state(localState(s))).toEqual({
      spec: { path: SPEC },
      tickets: {
        '1': { blocked_by: [], file: `${issues}/01-store.md`, status: 'merged', title: 'Widget store' },
        '2': { blocked_by: [1], file: `${issues}/02-list.md`, status: 'todo', title: 'Widget list' },
        '3': { blocked_by: [1, 2], file: `${issues}/03-search.md`, status: 'todo', title: 'Widget search' },
        '4': {
          blocked_by: [3],
          external: [{ ref: 'acme/gadgets#5' }, { ref: '9' }],
          file: `${issues}/04-live.md`,
          status: 'todo',
          title: 'Live check',
        },
      },
      title: 'Widgets spec',
      version: 1,
    })
    expect(s.tracker.calls).toEqual([])
  })

  it('reads each form a Blocked by takes: list items, several numbers in one, emphasis and empty entries', async () => {
    const s = await seeded()
    await localSpec(s, {
      '06-mixed.md': '# Mixed\n\nBlocked by:\n\n* `#2 and #3`\n* **Widget list**, ;\n1. 01-store.md\n\nNext paragraph\n- not a blocker\n',
      '07-loose.md': '# Loose\n\n## Blocked by\n\nSee the spec.\n\n- not a blocker\n',
    })

    await s.run('init', '--spec', SPEC)

    const tickets = (await s.state(localState(s))).tickets as Record<string, Record<string, unknown>>
    expect(tickets['6']).toMatchObject({ blocked_by: [2, 3, 1], title: 'Mixed' })
    expect(tickets['6']).not.toHaveProperty('external')
    expect(tickets['7']).toMatchObject({ blocked_by: [], title: 'Loose' })
  })

  it('reads a spec path against the working directory, so every spelling names one state', async () => {
    const s = await seeded()
    await localSpec(s)
    const runtime = defaultBuildTrackerRuntime({
      cwd: join(s.root, 'docs', 'specs'),
      env: { XDG_STATE_HOME: s.home },
      issues: s.tracker,
      repositories: s.repositories,
    })

    const relativeSpec = JSON.parse((await runBuildTrackerCli(['path', '--spec', 'widgets/spec.md'], runtime)).stdout) as Record<string, string>
    const absolute = JSON.parse((await s.run('path', '--spec', join(s.root, SPEC))).stdout) as Record<string, string>

    expect(relativeSpec).toEqual({ page: join(s.home, ...LOCAL_DIR, 'page.html'), spec: SPEC, state: localState(s), status: 'path' })
    expect(absolute).toEqual(relativeSpec)
  })

  it('names the page after the spec file and lists each ticket with its blockers', async () => {
    const s = await seeded()
    await localSpec(s)
    await s.run('init', '--spec', SPEC)
    const page = join(s.home, ...LOCAL_DIR, 'page.html')

    await s.run('render', '--state', localState(s), '--out', page)
    const html = await readFile(page, 'utf8')

    expect(html).toContain('<title>spec build tracker</title>')
    expect(html).toContain('#2</a> <span class="t">Widget list</span>')
    expect(html).toContain('blocked by #3, acme/gadgets#5 (other spec), 9 (other spec)</span>')
  })

  it("links each ticket to its file on the remote's file view", async () => {
    const s = await seeded()
    await localSpec(s)
    await s.run('init', '--spec', SPEC)
    const page = join(s.home, ...LOCAL_DIR, 'page.html')

    await s.run('render', '--state', localState(s), '--out', page)

    expect(await readFile(page, 'utf8')).toContain(
      '<a href="https://github.com/acme/widgets/blob/HEAD/docs/specs/widgets/issues/02-list.md">#2</a>',
    )
  })

  it('leaves the tickets unlinked, with no local path in the page, when the repository has no remote', async () => {
    const s = await seeded()
    await localSpec(s)
    s.repositories.roots[0] = { remote: undefined, root: s.root }
    const init = JSON.parse((await s.run('init', '--spec', SPEC)).stdout) as Record<string, string>

    await s.run('render', '--state', String(init.state), '--out', String(init.page))

    const page = await readFile(String(init.page), 'utf8')
    expect(page).toContain('<span class="n">#2</span> <span class="t">Widget list</span>')
    expect(page).not.toContain(s.root)
  })

  it('leaves the tickets unlinked, with no local path in the page, when the remote is on a host whose file view it does not know', async () => {
    const s = await seeded()
    await localSpec(s)
    s.repositories.roots[0] = { remote: { host: 'git.example.com', owner: 'acme', repo: 'widgets' }, root: s.root }

    const result = JSON.parse((await s.run('refresh', '--spec', SPEC)).stdout) as Record<string, string>

    const page = await readFile(String(result.page), 'utf8')
    expect(page).toContain('<span class="n">#2</span> <span class="t">Widget list</span>')
    expect(page).not.toContain(s.root)
  })

  it("leaves the tickets unlinked when rendered outside the spec's repository", async () => {
    const s = await seeded()
    await localSpec(s)
    await s.run('init', '--spec', SPEC)
    const elsewhere = defaultBuildTrackerRuntime({ cwd: s.home, env: { XDG_STATE_HOME: s.home }, issues: s.tracker, repositories: s.repositories })
    const page = join(s.home, 'p.html')

    await runBuildTrackerCli(['render', '--state', localState(s), '--out', page], elsewhere)

    expect(await readFile(page, 'utf8')).toContain('<span class="n">#2</span> <span class="t">Widget list</span>')
  })

  it('shows a local ticket with no file as its number without a link', async () => {
    const s = await seeded()
    await localSpec(s)
    await s.run('init', '--spec', SPEC)
    const state = (await s.state(localState(s))) as { tickets: Record<string, Record<string, unknown>> }
    delete state.tickets['2']?.file
    await writeFile(localState(s), JSON.stringify(state))
    const page = join(s.home, 'p.html')

    await s.run('render', '--state', localState(s), '--out', page)

    expect(await readFile(page, 'utf8')).toContain('<span class="n">#2</span> <span class="t">Widget list</span>')
  })

  it('syncs a local spec: a new ticket file is added and a ticket marked done turns merged', async () => {
    const s = await seeded()
    await localSpec(s)
    await s.run('init', '--spec', SPEC, '--milestone', 'Live=4')
    await s.run('set', '--state', localState(s), '3=running:on it')
    const issues = join(s.root, 'docs', 'specs', 'widgets', 'issues')
    await writeFile(join(issues, '02-list.md'), '# 02 - Widget list\n\nStatus: Closed\n\n**Blocked by:** 01\n')
    await writeFile(join(issues, '05-docs.md'), '# Docs\n\nBlocked by: 4\n')

    const result = await s.run('sync', '--spec', SPEC)

    expect(JSON.parse(result.stdout)).toMatchObject({ added: [5], merged: [2], spec: SPEC, status: 'synced', tickets: 5 })
    const state = await s.state(localState(s))
    const tickets = state.tickets as Record<string, Record<string, unknown>>
    expect(tickets['3']).toMatchObject({ note: 'on it', status: 'running' })
    expect(tickets['5']).toEqual({ blocked_by: [4], file: 'docs/specs/widgets/issues/05-docs.md', status: 'todo', title: 'Docs' })
    expect(state.milestones).toEqual([{ closes: 4, name: 'Live' }])
  })

  it('names a spec in a repository without a remote after its main working tree', async () => {
    const s = await seeded()
    await localSpec(s)
    s.repositories.roots[0] = { remote: undefined, root: s.root }

    const result = await s.run('path', '--spec', SPEC)

    const parent = (s.root.split('/').at(-2) ?? '').toLowerCase()
    expect(JSON.parse(result.stdout)).toMatchObject({
      state: join(s.home, 'agent-kit', 'build-tracker', 'local', parent, 'widgets', 'docs-specs-widgets-spec-md-7f62ea6e', 'state.json'),
    })
  })

  it('takes the spec title from its file name when it has no heading', async () => {
    const s = await seeded()
    await localSpec(s)
    await writeFile(join(s.root, SPEC), 'no heading here\n')

    await s.run('init', '--spec', SPEC)

    expect((await s.state(localState(s))).title).toBe('spec.md')
  })

  it.each([
    [
      'a ticket file with no number prefix',
      { 'readme.md': '# Read me\n' },
      'docs/specs/widgets/issues/readme.md has no ticket number in front of its name; name each ticket file like 01-readme.md',
    ],
    [
      'a ticket file numbered 0',
      { '00-zero.md': '# Zero\n' },
      'docs/specs/widgets/issues/00-zero.md has no ticket number in front of its name; name each ticket file like 01-00-zero.md',
    ],
    [
      'two ticket files with one number',
      { '2-again.md': '# Again\n' },
      'docs/specs/widgets/issues/02-list.md and docs/specs/widgets/issues/2-again.md both have ticket number 2',
    ],
  ])('refuses %s with a clear error and writes no state', async (_name, extra, message) => {
    const s = await seeded()
    await localSpec(s, extra)

    const result = await s.run('init', '--spec', SPEC)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code: 'invalid-ticket-file', message, status: 'error' })
    await expect(s.state(localState(s))).rejects.toThrow(/ENOENT/)
  })

  it('refuses a spec with no issues/ folder beside it', async () => {
    const s = await seeded()
    await mkdir(join(s.root, 'docs'))
    await writeFile(join(s.root, 'docs', 'lonely.md'), '# Lonely\n')

    const result = await s.run('init', '--spec', 'docs/lonely.md')

    expect(JSON.parse(result.stdout)).toEqual({
      code: 'invalid-ticket-file',
      message: 'there is no issues/ folder beside docs/lonely.md',
      status: 'error',
    })
  })

  it('refuses a spec that is no issue reference and no readable file', async () => {
    const s = await seeded()

    const result = await s.run('path', '--spec', 'docs/missing.md')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'invalid-spec',
      message: 'docs/missing.md is neither an issue reference (#<number>, owner/repo#<number> or an issue URL) nor a readable spec file',
      status: 'error',
    })
  })

  it('refuses a spec file outside any git repository', async () => {
    const s = await seeded()
    await writeFile(join(s.home, '..', 'loose.md'), '# Loose\n')

    const result = await s.run('path', '--spec', join(s.home, '..', 'loose.md'))

    expect(JSON.parse(result.stdout)).toEqual({
      code: 'invalid-spec',
      message: `${join(s.home, '..', 'loose.md')} is in no git repository`,
      status: 'error',
    })
  })
})

describe('build-tracker blockers outside the spec', () => {
  /** #3 also waits on an issue of another spec in acme/gadgets, and on #7, an issue of acme/widgets that is no ticket of this spec. */
  function crossSpec(gadget: 'open' | 'closed'): Record<string, FakeIssue> {
    return {
      ...widgetIssues(),
      '#2': { state: 'closed', title: 'widget list' },
      '#3': { blockedBy: [1, 2, 'acme/gadgets#5', 7], title: 'widget search' },
      '#7': { state: 'closed', title: 'another spec' },
      'acme/gadgets#5': { state: gadget, title: 'gadget' },
    }
  }

  async function initAndRender(s: Seeded): Promise<string> {
    const init = JSON.parse((await s.run('init', '--spec', '#10')).stdout) as Record<string, string>
    await s.run('render', '--state', String(init.state), '--out', String(init.page))
    return readFile(String(init.page), 'utf8')
  }

  it('keeps them per ticket as external blockers with their state', async () => {
    const s = await seeded(crossSpec('open'))

    await s.run('init', '--spec', '#10')

    expect(((await s.state()).tickets as Record<string, unknown>)['3']).toEqual({
      blocked_by: [1, 2],
      external: [
        { ref: 'acme/gadgets#5', state: 'open' },
        { ref: 'acme/widgets#7', state: 'closed' },
      ],
      status: 'todo',
      title: 'widget search',
    })
  })

  it('shows a ticket Blocked while an external blocker is open, and names it as another spec on the page', async () => {
    const s = await seeded(crossSpec('open'))

    const page = await initAndRender(s)

    expect(page).toMatch(/<li><span class="pill blocked">Blocked<\/span><a href="[^"]*\/issues\/3">/)
    expect(page).toContain('blocked by #1, #2, acme/gadgets#5 (other spec, open), acme/widgets#7 (other spec, closed)</span>')
  })

  it('draws each external blocker as a dashed node with a dotted edge', async () => {
    const s = await seeded(crossSpec('open'))

    const page = await initAndRender(s)

    const graph = graphSource(page)
    expect(graph).toContain('x1["acme/gadgets#35;5 · other spec"]:::external')
    expect(graph).toContain('x2["acme/widgets#35;7 · other spec"]:::external')
    expect(graph).toContain('x1 -.-> t3')
    expect(graph).toContain('x2 -.-> t3')
    expect(graph).toMatch(/classDef external [^\n]*stroke-dasharray/)
  })

  it('shows the ticket Ready once every external blocker is closed', async () => {
    const s = await seeded(crossSpec('closed'))

    const page = await initAndRender(s)

    expect(page).toMatch(/<li><span class="pill ready">Ready<\/span><a href="[^"]*\/issues\/3">/)
  })

  it('lets an external blocker whose state is unknown leave Ready and Blocked alone', async () => {
    const s = await seeded(crossSpec('open'))
    const init = JSON.parse((await s.run('init', '--spec', '#10')).stdout) as Record<string, string>
    const state = (await s.state()) as { tickets: Record<string, Record<string, unknown>> }
    state.tickets['3'] = { ...state.tickets['3'], external: ['acme/gadgets#5'] }
    state.tickets['4'] = { ...state.tickets['4'], blocked_by: [], external: [{ ref: 'acme/gadgets#6' }] }
    await writeFile(String(init.state), JSON.stringify(state))

    await s.run('render', '--state', String(init.state), '--out', String(init.page))
    const page = await readFile(String(init.page), 'utf8')

    expect(page).toMatch(/<li><span class="pill ready">Ready<\/span><a href="[^"]*\/issues\/3">/)
    expect(page).toMatch(/<li><span class="pill ready">Ready<\/span><a href="[^"]*\/issues\/4">/)
    expect(page).toContain('blocked by acme/gadgets#6 (other spec)</span>')
  })

  it('draws one node for an external blocker that several tickets wait on', async () => {
    const s = await seeded({
      ...widgetIssues(),
      '#2': { blockedBy: ['acme/gadgets#5'], title: 'widget list' },
      '#3': { blockedBy: ['acme/gadgets#5'], title: 'widget search' },
    })

    const graph = graphSource(await initAndRender(s))

    expect(graph.match(/:::external/g)).toHaveLength(1)
    expect(graph).toContain('x1 -.-> t2')
    expect(graph).toContain('x1 -.-> t3')
  })

  it.each([
    ['not a list', 'x', 'ticket 3: external must be a list of blocker references'],
    ['an empty reference', [''], 'ticket 3: external must be a list of blocker references'],
    ['an entry without a reference', [{ state: 'open' }], 'ticket 3: external must be a list of blocker references'],
    ['an unknown state', [{ ref: 'a/b#1', state: 'gone' }], "ticket 3: an external blocker's state must be open or closed"],
  ])('refuses a state whose external blockers are %s', async (_name, external, reason) => {
    const s = await seeded()
    const init = JSON.parse((await s.run('init', '--spec', '#10')).stdout) as Record<string, string>
    const state = (await s.state()) as { tickets: Record<string, Record<string, unknown>> }
    state.tickets['3'] = { ...state.tickets['3'], external }
    await writeFile(String(init.state), JSON.stringify(state))

    const result = await s.run('render', '--state', String(init.state), '--out', String(init.page))

    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state' })
    expect(result.stderr).toContain(`unreadable: ${reason}; delete it and run init again`)
  })

  it('sync updates external blockers and drops them when the issue tracker no longer lists them', async () => {
    const s = await seeded(crossSpec('open'))
    await s.run('init', '--spec', '#10')
    s.tracker.issues = { ...s.tracker.issues, 'acme/gadgets#5': { state: 'closed' } }

    await s.run('sync', '--spec', '#10')
    const tickets = (await s.state()).tickets as Record<string, Record<string, unknown>>
    expect(tickets['3']?.external).toEqual([
      { ref: 'acme/gadgets#5', state: 'closed' },
      { ref: 'acme/widgets#7', state: 'closed' },
    ])

    s.tracker.issues = { ...s.tracker.issues, '#3': { blockedBy: [1, 2], title: 'widget search' } }
    await s.run('sync', '--spec', '#10')
    expect(((await s.state()).tickets as Record<string, Record<string, unknown>>)['3']).not.toHaveProperty('external')
  })
})

describe('build-tracker path', () => {
  it('prints the state file and page paths, with or without a state', async () => {
    const s = await seeded()

    const result = await s.run('path', '--spec', '#10')

    expect(result.exitCode).toBe(0)
    expect(JSON.parse(result.stdout)).toEqual({
      page: join(s.home, ...WIDGETS_DIR, 'page.html'),
      spec: 'acme/widgets#10',
      state: join(s.home, ...WIDGETS_DIR, 'state.json'),
      status: 'path',
    })
    expect(s.tracker.calls).toEqual([])
  })

  it.each([
    ['XDG_STATE_HOME is unset', { HOME: '/home/ann' }],
    ['XDG_STATE_HOME is relative', { HOME: '/home/ann', XDG_STATE_HOME: 'state' }],
    ['XDG_STATE_HOME is empty', { HOME: '/home/ann', XDG_STATE_HOME: '' }],
  ])('uses ~/.local/state when %s', async (_name, env) => {
    const runtime = defaultBuildTrackerRuntime({ env, issues: new FakeTracker({}) })

    const result = await runBuildTrackerCli(['path', '--spec', 'acme/widgets#10'], runtime)

    expect(JSON.parse(result.stdout)).toMatchObject({
      state: '/home/ann/.local/state/agent-kit/build-tracker/github.com/acme/widgets/10/state.json',
    })
  })

  it('names the same place for every spelling of the spec, whatever its case', async () => {
    const s = await seeded()

    const upper = JSON.parse((await s.run('path', '--spec', 'Acme/Widgets#10')).stdout) as Record<string, unknown>

    expect(upper.state).toBe(join(s.home, ...WIDGETS_DIR, 'state.json'))
    expect(upper.spec).toBe('Acme/Widgets#10')
  })
})

describe('build-tracker state places', () => {
  it('gives two local specs whose paths read alike their own state', async () => {
    const s = await seeded()
    await mkdir(join(s.root, 'docs', 'specs'), { recursive: true })
    await writeFile(join(s.root, 'docs', 'specs', 'v1.2.md'), '# Version one point two\n')
    await writeFile(join(s.root, 'docs', 'specs', 'v1_2.md'), '# Version one underscore two\n')
    const dotted = JSON.parse((await s.run('path', '--spec', 'docs/specs/v1.2.md')).stdout) as Record<string, string>
    const underscored = JSON.parse((await s.run('path', '--spec', 'docs/specs/v1_2.md')).stdout) as Record<string, string>
    expect(underscored.state).not.toBe(dotted.state)
  })

  it('refuses to name a state when neither XDG_STATE_HOME nor HOME is an absolute path', async () => {
    const runtime = defaultBuildTrackerRuntime({ env: { HOME: 'relative' }, issues: new FakeTracker({}) })
    const result = await runBuildTrackerCli(['path', '--spec', 'acme/widgets#10'], runtime)
    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'no-state-home', status: 'error' })
  })
})

describe('build-tracker refresh', () => {
  it('leaves the state as it was when the page cannot be written', async () => {
    const s = await seeded()
    await s.run('refresh', '--spec', '#10')
    const state = await s.state()
    await rm(pagePath(s))
    await mkdir(pagePath(s))
    const result = await s.run('refresh', '--spec', '#10', '--landed', '2')
    expect(result.exitCode).toBe(1)
    expect(await s.state()).toEqual(state)
  })

  const statePath = (s: Seeded): string => join(s.home, ...WIDGETS_DIR, 'state.json')
  const pagePath = (s: Seeded): string => join(s.home, ...WIDGETS_DIR, 'page.html')

  it('on a spec with no state seeds it like init, marks the landed tickets merged, renders the page and prints its path', async () => {
    const s = await seeded()

    const result = await s.run('refresh', '--spec', '#10', '--landed', '#2')

    expect(result.stderr).toBe('')
    expect(result.exitCode).toBe(0)
    expect(JSON.parse(result.stdout)).toEqual({
      added: [1, 2, 3, 4],
      counts: { blocked: 1, merged: 2, ready: 1 },
      merged: [1, 2],
      page: pagePath(s),
      ran: 'init',
      spec: 'acme/widgets#10',
      state: statePath(s),
      status: 'refreshed',
      tickets: 4,
    })
    expect(await s.state()).toEqual({
      spec: 'acme/widgets#10',
      tickets: {
        '1': { blocked_by: [], status: 'merged', title: 'widget store' },
        '2': { blocked_by: [1], status: 'merged', title: 'widget list' },
        '3': { blocked_by: [1, 2], status: 'todo', title: 'widget search' },
        '4': { blocked_by: [3], status: 'todo', title: 'live check on widgets' },
      },
      title: 'Spec: widgets',
      version: 1,
    })
    const page = await readFile(pagePath(s), 'utf8')
    expect(page).toContain('<a href="https://github.com/acme/widgets/issues/2">#2</a>')
    expect(page).toContain('2026-11-03 12:00 UTC')
  })

  it('on a spec with a state syncs it, marks the landed tickets merged, keeps what the agent set and renders', async () => {
    const s = await seeded()
    await s.run('init', '--spec', '#10', '--milestone', 'Live=4')
    await s.run('set', '--state', statePath(s), '2=review:merging now', '3=owner:needs a key')
    s.tracker.issues = {
      ...s.tracker.issues,
      '#10': { subIssues: [1, 2, 3, 4, 5], title: 'Spec: widgets' },
      '#4': { blockedBy: [3], state: 'closed', title: 'live check on widgets' },
      '#5': { blockedBy: [4], title: 'widget docs' },
    }

    const result = await s.run('refresh', '--spec', 'acme/widgets#10', '--landed', '2', '--landed', '#1')

    expect(result.exitCode).toBe(0)
    expect(JSON.parse(result.stdout)).toEqual({
      added: [5],
      counts: { merged: 3, owner: 1, ready: 1 },
      merged: [2, 4],
      page: pagePath(s),
      ran: 'sync',
      spec: 'acme/widgets#10',
      state: statePath(s),
      status: 'refreshed',
      tickets: 5,
    })
    expect(await s.state()).toEqual({
      milestones: [{ closes: 4, name: 'Live' }],
      spec: 'acme/widgets#10',
      tickets: {
        '1': { blocked_by: [], status: 'merged', title: 'widget store' },
        '2': { blocked_by: [1], note: 'merging now', status: 'merged', title: 'widget list' },
        '3': { blocked_by: [1, 2], note: 'needs a key', status: 'owner', title: 'widget search' },
        '4': { blocked_by: [3], status: 'merged', title: 'live check on widgets' },
        '5': { blocked_by: [4], status: 'todo', title: 'widget docs' },
      },
      title: 'Spec: widgets',
      version: 1,
    })
    const page = await readFile(pagePath(s), 'utf8')
    expect(page).toContain('<a href="https://github.com/acme/widgets/issues/5">#5</a>')
    expect(page).toContain('Live')
  })

  it('gives the same state and page when nothing changed since the last refresh', async () => {
    const s = await seeded()
    await s.run('refresh', '--spec', '#10', '--landed', '2')
    const state = await s.state()
    const page = await readFile(pagePath(s), 'utf8')

    const again = await s.run('refresh', '--spec', '#10', '--landed', '2')

    expect(JSON.parse(again.stdout)).toMatchObject({ added: [], merged: [], ran: 'sync' })
    expect(await s.state()).toEqual(state)
    expect(await readFile(pagePath(s), 'utf8')).toBe(page)
  })

  it('reads a landed local ticket by the number of its file', async () => {
    const s = await seeded()
    await mkdir(join(s.root, 'specs', 'issues'), { recursive: true })
    await writeFile(join(s.root, 'specs', 'spec.md'), '# Local widgets\n')
    await writeFile(join(s.root, 'specs', 'issues', '01-store.md'), '# Widget store\n')
    await writeFile(join(s.root, 'specs', 'issues', '02-list.md'), '# Widget list\n\n## Blocked by\n\n- 01\n')

    const result = await s.run('refresh', '--spec', 'specs/spec.md', '--landed', '01')

    const output = JSON.parse(result.stdout) as { state: string; merged: number[]; counts: object }
    expect(output).toMatchObject({ counts: { merged: 1, ready: 1 }, merged: [1], ran: 'init', status: 'refreshed' })
    const tickets = (await s.state(output.state)).tickets as Record<string, { status: string }>
    expect([tickets['1']?.status, tickets['2']?.status]).toEqual(['merged', 'todo'])
  })

  it.each([
    ['a ticket outside the spec', '9', 'unknown-ticket', "ticket 9 is not one of the spec's tickets"],
    ['a name that is no ticket number', 'widget-list', 'invalid-ticket', 'widget-list is not a ticket number'],
  ])('refuses %s as landed and writes neither a state nor a page', async (_name, landed, code, message) => {
    const s = await seeded()

    const result = await s.run('refresh', '--spec', '#10', '--landed', '2', '--landed', landed)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code, message, status: 'error' })
    await expect(s.state()).rejects.toThrow(/ENOENT/)
    await expect(readFile(pagePath(s), 'utf8')).rejects.toThrow(/ENOENT/)
  })

  it('leaves an existing state and page as they were when it refuses', async () => {
    const s = await seeded()
    await s.run('refresh', '--spec', '#10')
    const state = await s.state()
    const page = await readFile(pagePath(s), 'utf8')
    s.tracker.issues = { ...s.tracker.issues, '#2': { blockedBy: [1], state: 'closed', title: 'widget list' } }

    const result = await s.run('refresh', '--spec', '#10', '--landed', '9')

    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unknown-ticket', status: 'error' })
    expect(await s.state()).toEqual(state)
    expect(await readFile(pagePath(s), 'utf8')).toBe(page)
  })

  it('reports an issue tracker it cannot read and writes nothing', async () => {
    const s = await seeded()
    s.tracker.failure = 'HTTP 502'

    const result = await s.run('refresh', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toEqual({
      code: 'tracker-unreachable',
      message: 'cannot read the tickets of acme/widgets#10: HTTP 502',
      status: 'error',
    })
    await expect(s.state()).rejects.toThrow(/ENOENT/)
  })

  it('refuses an unreadable state, saying to delete it and run init again, and reads no tickets', async () => {
    const s = await seeded()
    await mkdir(join(s.home, ...WIDGETS_DIR), { recursive: true })
    await writeFile(statePath(s), '{ not json')

    const result = await s.run('refresh', '--spec', '#10')

    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state', status: 'error' })
    expect(result.stderr).toMatch(new RegExp(`^build-tracker refresh: the state file at ${statePath(s)} is unreadable: .*; delete it and run init again\n$`))
    expect(s.tracker.calls).toEqual([])
  })

  it.each([
    ['--spec twice', ['--spec', '#10', '--spec', '#10']],
    ['no --spec', ['--landed', '2']],
    ['a --landed without a value', ['--spec', '#10', '--landed']],
    ['a stray argument', ['--spec', '#10', '2']],
  ])('prints its usage and exits 2 for %s', async (_name, args) => {
    const s = await seeded()

    expect((await s.run('refresh', ...args)).exitCode).toBe(2)
  })

  it('names refresh in its usage', async () => {
    const s = await seeded()

    expect((await s.run('--help')).stdout).toContain('build-tracker refresh --spec SPEC [--landed <ticket> ...]')
  })
})
