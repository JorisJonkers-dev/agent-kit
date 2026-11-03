import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { BUILD_TRACKER_USAGE, defaultBuildTrackerRuntime, runBuildTrackerCli, type BuildTrackerCliResult } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

const NOON = new Date('2026-11-03T12:00:00Z')

interface Workspace {
  readonly dir: string
  readonly state: string
  readonly out: string
  writeState(value: unknown): Promise<void>
  run(...argv: string[]): Promise<BuildTrackerCliResult>
  page(): Promise<string>
}

async function workspace(now: () => Date = () => NOON): Promise<Workspace> {
  const dir = await mkdtemp(join(tmpdir(), 'build-tracker-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const state = join(dir, 'state.json')
  const out = join(dir, 'tracker.html')
  const runtime = defaultBuildTrackerRuntime({ now })
  return {
    dir,
    out,
    page: () => readFile(out, 'utf8'),
    run: (...argv) => runBuildTrackerCli(argv, runtime),
    state,
    writeState: (value) => writeFile(state, `${JSON.stringify(value, null, 2)}\n`),
  }
}

/** A small spec: #2 and #3 wait on #1, #4 closes the work as its live check. */
function smallState(): Record<string, unknown> {
  return {
    spec: 'acme/widgets#10',
    subtitle: 'integration branch feat/widgets',
    tickets: {
      '1': { blocked_by: [], status: 'merged', title: 'widget store', note: 'abc1234' },
      '2': { blocked_by: [1], status: 'todo', title: 'widget list' },
      '3': { blocked_by: [1, 2], status: 'todo', title: 'widget search' },
      '4': { blocked_by: [3], live: true, status: 'todo', title: 'live check on widgets' },
    },
    title: 'Widgets build',
    version: 1,
  }
}

/** The status label the page shows for ticket `n`, read from its list item. */
function shownAs(page: string, n: number): string | undefined {
  const item = new RegExp(`<li><span class="pill [a-z]+">([^<]+)</span><a href="[^"]*/issues/${String(n)}">`).exec(page)
  return item?.[1]
}

/** The Mermaid source as Mermaid reads it: the graph block's text, its HTML entities decoded. */
function graphSource(page: string): string {
  const block = /<pre class="mermaid">([\s\S]*?)<\/pre>/.exec(page)?.[1] ?? ''
  return block.replaceAll('&quot;', '"').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&')
}

describe('build-tracker render', () => {
  it('writes one page listing every ticket with its issue link and title', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    const result = await w.run('render', '--state', w.state, '--out', w.out)

    expect(result.exitCode).toBe(0)
    const page = await w.page()
    expect(page).toContain('<a href="https://github.com/acme/widgets/issues/2">#2</a>')
    expect(page).toContain('widget list')
    expect(page).toContain('<h1>Widgets build</h1>')
  })

  it('gives the same page for the same state, apart from the updated time', async () => {
    let now = NOON
    const w = await workspace(() => now)
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)
    const first = await w.page()
    await w.run('render', '--state', w.state, '--out', w.out)
    expect(await w.page()).toBe(first)
    now = new Date('2026-11-04T08:30:00Z')
    await w.run('render', '--state', w.state, '--out', w.out)
    const later = await w.page()

    expect(first).toContain('updated 2026-11-03 12:00 UTC')
    expect(later).toContain('updated 2026-11-04 08:30 UTC')
    expect(later.replace('2026-11-04 08:30', '2026-11-03 12:00')).toBe(first)
  })

  it('shows a todo ticket as Ready once every blocker is merged, and as Blocked before', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)
    const page = await w.page()

    expect(shownAs(page, 2)).toBe('Ready')
    expect(shownAs(page, 3)).toBe('Blocked')
    expect(shownAs(page, 4)).toBe('Blocked')
  })

  it('shows running as Building, review as Merging, merged as Merged and owner as Needs you', async () => {
    const w = await workspace()
    const state = smallState()
    await w.writeState({
      ...state,
      tickets: {
        '1': { blocked_by: [], status: 'running', title: 'one' },
        '2': { blocked_by: [], status: 'review', title: 'two' },
        '3': { blocked_by: [], status: 'merged', title: 'three' },
        '4': { blocked_by: [], status: 'owner', title: 'four', note: 'needs a token' },
      },
    })

    await w.run('render', '--state', w.state, '--out', w.out)
    const page = await w.page()

    expect([1, 2, 3, 4].map((n) => shownAs(page, n))).toEqual(['Building', 'Merging', 'Merged', 'Needs you'])
  })

  it('has a summary strip with a count per shown status', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)
    const strip = /<div class="strip">([\s\S]*?)<\/div>/.exec(await w.page())?.[1] ?? ''

    expect([...strip.matchAll(/<span class="pill ([a-z]+)">([^<]+)<\/span>/g)].map((m) => [m[1], m[2]])).toEqual([
      ['merged', 'Merged 1'],
      ['ready', 'Ready 1'],
      ['blocked', 'Blocked 2'],
    ])
  })

  it('draws a Mermaid dependency graph with one node per ticket, one edge per blocker and one colour per status', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)
    const graph = graphSource(await w.page())

    expect(graph).toContain('flowchart LR')
    expect(graph).toContain('t1["#1 widget store"]:::merged')
    expect(graph).toContain('t2["#2 widget list"]:::ready')
    expect(graph).toContain('t4["#4 live check on widgets"]:::blocked')
    expect(graph.match(/-->/g)).toHaveLength(4)
    expect(graph).toContain('t1 --> t3')
    expect(graph).toContain('t2 --> t3')
    const fills = [...graph.matchAll(/classDef ([a-z]+) fill:(#[0-9a-f]+)/g)].map((m) => [m[1], m[2]])
    expect(fills.map(([name]) => name).sort()).toEqual(['blocked', 'merged', 'owner', 'ready', 'review', 'running'])
    expect(new Set(fills.map(([, fill]) => fill)).size).toBe(6)
  })

  it('escapes titles in the graph and the lists, and shortens long graph labels', async () => {
    const w = await workspace()
    await w.writeState({
      ...smallState(),
      tickets: {
        '1': { blocked_by: [], status: 'todo', title: 'after #2; say "hi" & <script>x</script>' },
        '2': { blocked_by: [], status: 'todo', title: 'a title far longer than any graph node should ever have to show' },
        '3': { blocked_by: [], status: 'todo', title: 'two\nlines' },
      },
    })

    await w.run('render', '--state', w.state, '--out', w.out)
    const page = await w.page()

    expect(graphSource(page)).toContain('t3["#3 two lines"]')
    expect(page).not.toContain('<script>x')
    expect(page).toContain('<span class="t">after #2; say &quot;hi&quot; &amp; &lt;script&gt;x&lt;/script&gt;</span>')
    // Mermaid's own entity codes stand for # and " inside a node label.
    expect(graphSource(page)).toContain('t1["#1 after #35;2; say #quot;hi#quot; & <script>x</sc…"]')
    expect(graphSource(page)).toContain('t2["#2 a title far longer than any graph …"]')
  })

  it('lists each ticket in its section with its link, title, blockers, note and a live-check tag', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)
    const page = await w.page()

    expect(page).toContain(
      '<li><span class="pill merged">Merged</span><a href="https://github.com/acme/widgets/issues/1">#1</a> ' +
        '<span class="t">widget store</span><span class="meta">blocked by none · abc1234</span></li>',
    )
    expect(page).toContain(
      '<a href="https://github.com/acme/widgets/issues/3">#3</a> <span class="t">widget search</span>' +
        '<span class="meta">blocked by #1, #2</span></li>',
    )
    expect(page).toContain(
      '<span class="t">live check on widgets</span> <span class="live">live check</span><span class="meta">blocked by #3</span>',
    )
  })
})

/** Each section's heading and the tickets it lists, in page order. */
function sections(page: string): [string, number[]][] {
  return [...page.matchAll(/<section><h2>([^<]+) <small>[^<]*<\/small><\/h2>([\s\S]*?)<\/section>/g)].map((m) => [
    String(m[1]),
    [...String(m[2]).matchAll(/\/issues\/(\d+)"/g)].map((n) => Number(n[1])),
  ])
}

describe('build-tracker milestones', () => {
  /** #1, #2, #3 (closes A); #1, #4, #5 (closes B, also after #3); #6 on its own. */
  function milestoneState(milestones: unknown): Record<string, unknown> {
    return {
      ...smallState(),
      milestones,
      tickets: {
        '1': { blocked_by: [], status: 'merged', title: 'base' },
        '2': { blocked_by: [1], status: 'todo', title: 'second' },
        '3': { blocked_by: [2], live: true, status: 'todo', title: 'live check A' },
        '4': { blocked_by: [1], status: 'todo', title: 'fourth' },
        '5': { blocked_by: [3, 4], live: true, status: 'todo', title: 'live check B' },
        '6': { blocked_by: [], status: 'owner', title: 'loose end' },
      },
    }
  }

  it('puts each ticket in the first milestone whose closing ticket depends on it, directly or through others', async () => {
    const w = await workspace()
    await w.writeState(
      milestoneState([
        { closes: 3, name: 'Milestone A' },
        { closes: 5, name: 'Milestone B' },
      ]),
    )

    await w.run('render', '--state', w.state, '--out', w.out)

    expect(sections(await w.page())).toEqual([
      ['Milestone A', [1, 2, 3]],
      ['Milestone B', [4, 5]],
      ['Other', [6]],
    ])
  })

  it('counts the merged tickets of each milestone', async () => {
    const w = await workspace()
    await w.writeState(milestoneState([{ closes: 3, name: 'Milestone A' }]))

    await w.run('render', '--state', w.state, '--out', w.out)

    expect(await w.page()).toContain('<h2>Milestone A <small>1/3 merged</small></h2>')
  })

  it('leaves out the Other section when every ticket is in a milestone', async () => {
    const w = await workspace()
    const state = milestoneState([{ closes: 5, name: 'Everything' }])
    delete (state.tickets as Record<string, unknown>)['6']
    await w.writeState(state)

    await w.run('render', '--state', w.state, '--out', w.out)

    expect(sections(await w.page())).toEqual([['Everything', [1, 2, 3, 4, 5]]])
  })

  it('gives a state without milestones one section', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('render', '--state', w.state, '--out', w.out)

    expect(sections(await w.page())).toEqual([['Tickets', [1, 2, 3, 4]]])
  })
})

describe('build-tracker set', () => {
  async function stateOf(w: Workspace): Promise<Record<string, Record<string, unknown>>> {
    return (JSON.parse(await readFile(w.state, 'utf8')) as { tickets: Record<string, Record<string, unknown>> }).tickets
  }

  it('changes the named tickets in place and says what it changed', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    const result = await w.run('set', '--state', w.state, '2=running', '3=owner:needs the API key')

    expect(result.exitCode).toBe(0)
    expect(JSON.parse(result.stdout)).toEqual({
      changed: [
        { status: 'running', ticket: 2 },
        { status: 'owner', ticket: 3 },
      ],
      status: 'set',
    })
    const tickets = await stateOf(w)
    expect(tickets['2']).toEqual({ blocked_by: [1], status: 'running', title: 'widget list' })
    expect(tickets['3']).toEqual({ blocked_by: [1, 2], note: 'needs the API key', status: 'owner', title: 'widget search' })
    expect(tickets['4']).toEqual({ blocked_by: [3], live: true, status: 'todo', title: 'live check on widgets' })
  })

  it('keeps a note when none is given, removes it when the note is empty, and keeps colons inside a note', async () => {
    const w = await workspace()
    await w.writeState(smallState())

    await w.run('set', '--state', w.state, '1=review')
    expect((await stateOf(w))['1']?.note).toBe('abc1234')
    await w.run('set', '--state', w.state, '#1=merged:')
    expect((await stateOf(w))['1']).not.toHaveProperty('note')
    await w.run('set', '--state', w.state, '1=merged:see https://example.test/a:b')
    expect((await stateOf(w))['1']?.note).toBe('see https://example.test/a:b')
  })

  it('keeps every key of the state it does not change', async () => {
    const w = await workspace()
    const state: Record<string, unknown> = { ...smallState(), extra: { kept: true } }
    ;(state.tickets as Record<string, Record<string, unknown>>)['2'] = { blocked_by: [1], status: 'todo', title: 'widget list', url: 'x' }
    await w.writeState(state)

    await w.run('set', '--state', w.state, '2=merged')

    const written = JSON.parse(await readFile(w.state, 'utf8')) as Record<string, unknown>
    expect(written.extra).toEqual({ kept: true })
    expect(written.version).toBe(1)
    expect((written.tickets as Record<string, unknown>)['2']).toEqual({ blocked_by: [1], status: 'merged', title: 'widget list', url: 'x' })
  })

  it.each([
    ['an unknown ticket', ['2=running', '99=merged'], 'unknown-ticket', 'ticket 99 is not in the state'],
    ['an unknown status', ['2=running', '3=done'], 'unknown-status', 'done is not a status; use one of todo, running, review, merged, owner'],
    ['an argument that is no assignment', ['2=running', 'three'], 'invalid-assignment', 'three is not <ticket>=<status>[:note]'],
  ])('refuses %s, exits non-zero and leaves the file unchanged', async (_name, assignments, code, message) => {
    const w = await workspace()
    await w.writeState(smallState())
    const before = await readFile(w.state)

    const result = await w.run('set', '--state', w.state, ...assignments)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({ code, message, status: 'error' })
    expect(result.stderr).toBe(`build-tracker set: ${message}\n`)
    expect(await readFile(w.state)).toEqual(before)
  })

  it('replaces the state file through a rename, leaving no temporary file behind', async () => {
    const w = await workspace()
    await w.writeState(smallState())
    const before = await stat(w.state)

    await w.run('set', '--state', w.state, '2=running')

    // A rename puts a new file in place, so the inode changes; an in-place write keeps it.
    expect((await stat(w.state)).ino).not.toBe(before.ino)
    expect(await readdir(w.dir)).toEqual(['state.json'])
  })
})

describe('build-tracker writes', () => {
  it('writes the page through a rename too, creating its directory', async () => {
    const w = await workspace()
    await w.writeState(smallState())
    const out = join(w.dir, 'site', 'tracker.html')
    await w.run('render', '--state', w.state, '--out', out)
    const before = await stat(out)

    await w.run('render', '--state', w.state, '--out', out)

    expect((await stat(out)).ino).not.toBe(before.ino)
    expect(await readdir(join(w.dir, 'site'))).toEqual(['tracker.html'])
  })

  it('reports a page it cannot write and removes its temporary file', async () => {
    const w = await workspace()
    await w.writeState(smallState())
    // A directory where the page should go: the rename onto it fails.
    await mkdir(w.out)

    const result = await w.run('render', '--state', w.state, '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unexpected', status: 'error' })
    expect((await readdir(w.dir)).sort()).toEqual(['state.json', 'tracker.html'])
  })
})

describe('build-tracker state file errors', () => {
  it.each(['render', 'set'])('%s on a missing state file exits non-zero with an error that names init', async (command) => {
    const w = await workspace()
    const args = command === 'render' ? ['--out', w.out] : ['2=running']

    const result = await w.run(command, '--state', w.state, ...args)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toEqual({
      code: 'missing-state',
      message: `there is no state file at ${w.state}; create it with init`,
      status: 'error',
    })
    await expect(readdir(w.dir)).resolves.toEqual([])
  })

  const unreadable: [string, unknown, string][] = [
    ['not an object', [1, 2], 'the state is not a JSON object'],
    ['missing its version', { ...smallState(), version: undefined }, 'version must be 1, not missing'],
    ['of another version', { ...smallState(), version: 2 }, 'version must be 1, not 2'],
    ['with a bad spec reference', { ...smallState(), spec: '#10' }, 'spec must be a reference like owner/repo#123'],
    ['without a title', { ...smallState(), title: 3 }, 'title must be a string'],
    ['with a bad subtitle', { ...smallState(), subtitle: [] }, 'subtitle must be a string'],
    ['without tickets', { ...smallState(), tickets: [] }, 'tickets must be an object keyed by issue number'],
    ['with a ticket keyed by a name', { ...smallState(), tickets: { one: {} } }, 'ticket one: a ticket key must be its issue number'],
    ['with a ticket that is no object', { ...smallState(), tickets: { '1': 'x' } }, 'ticket 1 is not an object'],
    ['with bad blockers', { ...smallState(), tickets: { '1': { blocked_by: ['2'] } } }, 'ticket 1: blocked_by must be a list of issue numbers'],
    [
      'with an unknown status',
      { ...smallState(), tickets: { '1': { blocked_by: [], status: 'ready', title: 'x' } } },
      'ticket 1: status must be one of todo, running, review, merged, owner',
    ],
    [
      'with a bad live flag',
      { ...smallState(), tickets: { '1': { blocked_by: [], live: 'yes', status: 'todo', title: 'x' } } },
      'ticket 1: live must be true or false',
    ],
    [
      'with a bad note',
      { ...smallState(), tickets: { '1': { blocked_by: [], note: 7, status: 'todo', title: 'x' } } },
      'ticket 1: note must be a string',
    ],
    [
      'with a blocker outside the state',
      { ...smallState(), tickets: { '1': { blocked_by: [9], status: 'todo', title: 'x' } } },
      'ticket 1 is blocked by 9, which is not in the state',
    ],
    ['with milestones that are no list', { ...smallState(), milestones: {} }, 'milestones must be a list'],
    ['with a milestone that is no object', { ...smallState(), milestones: ['x'] }, 'milestone 1 is not an object'],
    ['with an unnamed milestone', { ...smallState(), milestones: [{ closes: 4 }] }, 'milestone 1: name must be a string'],
    [
      'with a milestone closed by a ticket outside the state',
      { ...smallState(), milestones: [{ closes: 4, name: 'A' }, { closes: 9, name: 'B' }] },
      'milestone 2: closes must name a ticket in the state',
    ],
  ]

  it.each(unreadable)('render and set refuse a state %s, naming init', async (_name, state, reason) => {
    const w = await workspace()
    await w.writeState(state)
    const before = await readFile(w.state)

    for (const args of [['render', '--state', w.state, '--out', w.out], ['set', '--state', w.state, '1=merged']]) {
      const result = await w.run(...args)
      expect(result.exitCode).toBe(1)
      expect(JSON.parse(result.stdout)).toEqual({
        code: 'unreadable-state',
        message: `the state file at ${w.state} is unreadable: ${reason}; fix it or create it again with init`,
        status: 'error',
      })
    }
    expect(await readFile(w.state)).toEqual(before)
    expect(await readdir(w.dir)).toEqual(['state.json'])
  })

  it('refuses a state file that is not JSON, naming init', async () => {
    const w = await workspace()
    await writeFile(w.state, '{"version": 1,')

    const result = await w.run('render', '--state', w.state, '--out', w.out)

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state', status: 'error' })
    expect(result.stderr).toMatch(
      /^build-tracker render: the state file at .* is unreadable: it is not JSON \(SyntaxError: .*\); fix it or create it again with init\n$/,
    )
  })

  it('refuses a state path it cannot read as a file, naming init', async () => {
    const w = await workspace()
    await mkdir(w.state)

    const result = await w.run('set', '--state', w.state, '1=merged')

    expect(result.exitCode).toBe(1)
    expect(JSON.parse(result.stdout)).toMatchObject({ code: 'unreadable-state', status: 'error' })
    expect(result.stderr).toMatch(/EISDIR.*; fix it or create it again with init\n$/)
  })
})

describe('build-tracker command line', () => {
  it('prints its usage for --help', async () => {
    const result = await runBuildTrackerCli(['--help'], defaultBuildTrackerRuntime())

    expect(result).toEqual({ exitCode: 0, stderr: '', stdout: BUILD_TRACKER_USAGE })
    expect(BUILD_TRACKER_USAGE).toContain('build-tracker render --state FILE --out FILE')
    expect(BUILD_TRACKER_USAGE).toContain('build-tracker set --state FILE <ticket>=<status>[:note] ...')
  })

  it.each([
    ['no command', []],
    ['an unknown command', ['publish', '--state', 's']],
    ['render without --out', ['render', '--state', 's']],
    ['render with a stray argument', ['render', '--state', 's', '--out', 'o', 'extra']],
    ['render with an unknown flag', ['render', '--state', 's', '--out', 'o', '--force', 'x']],
    ['render with a flag twice', ['render', '--state', 's', '--state', 't', '--out', 'o']],
    ['render with a flag and no value', ['render', '--out', 'o', '--state']],
    ['set without assignments', ['set', '--state', 's']],
    ['set without --state', ['set', '1=merged']],
  ])('prints its usage and exits 2 for %s', async (_name, argv) => {
    const result = await runBuildTrackerCli(argv, defaultBuildTrackerRuntime())

    expect(result).toEqual({ exitCode: 2, stderr: BUILD_TRACKER_USAGE, stdout: '' })
  })

  it('reads the real clock when none is given', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'build-tracker-'))
    cleanups.push(() => rm(dir, { force: true, recursive: true }))
    await writeFile(join(dir, 'state.json'), JSON.stringify(smallState()))
    const before = new Date()

    await runBuildTrackerCli(['render', '--state', join(dir, 'state.json'), '--out', join(dir, 'p.html')], defaultBuildTrackerRuntime())

    const after = new Date()
    const stamp = /updated (\d{4}-\d\d-\d\d \d\d:\d\d) UTC/.exec(await readFile(join(dir, 'p.html'), 'utf8'))?.[1]
    const minute = (time: Date) => time.toISOString().slice(0, 16).replace('T', ' ')
    expect([minute(before), minute(after)]).toContain(stamp)
  })
})
