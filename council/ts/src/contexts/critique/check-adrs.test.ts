import { cp, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../../test/critique/harness.js'
import { ADR_DIRECTORY } from './domain/decision-docs.js'
import type { CheckAdrsResult, Refusal } from './index.js'

/**
 * The deterministic decision-log checker (#226): numbering, status values,
 * link and supersede targets, and records in force that still cite a
 * superseded one, so no critic budget goes on what a script can settle.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function withLog(records: Readonly<Record<string, string>>): Promise<Harness> {
  const h = await harness()
  open.push(h)
  for (const [name, text] of Object.entries(records))
    await h.writeText(join(h.repo, 'docs', 'adr', name), text)
  return h
}

async function check(h: Harness): Promise<CheckAdrsResult | Refusal> {
  return h.commands.checkAdrs.run({ repo: h.repo })
}

async function checked(h: Harness): Promise<CheckAdrsResult> {
  const result = await check(h)
  if (result.status === 'refused') throw new Error(`check-adrs refused: ${result.message}`)
  return result
}

describe('check-adrs on a well-formed log', () => {
  it('passes, listing each record with its number and status', async () => {
    const h = await withLog({
      '0001-nightly-job.md':
        '# The nightly job frobnicates widgets\n\nWe frobnicate widgets nightly.\n',
      '0002-window.md':
        '# Widgets keep a window\n\nStatus: accepted\n\nThe window follows [ADR-0001](0001-nightly-job.md).\n',
    })

    expect(await checked(h)).toEqual({
      cites_superseded: [],
      numbering: { duplicates: [], gaps: [], unnumbered: [] },
      records: [
        {
          in_force: true,
          number: 1,
          path: 'docs/adr/0001-nightly-job.md',
          status: null,
          title: 'The nightly job frobnicates widgets',
        },
        {
          in_force: true,
          number: 2,
          path: 'docs/adr/0002-window.md',
          status: 'accepted',
          title: 'Widgets keep a window',
        },
      ],
      status: 'passed',
      unknown_statuses: [],
      unresolved_links: [],
    })
  })
})

describe('check-adrs on a log with every kind of structural error', () => {
  it('reports a gap, a duplicate number, an unknown status, an unresolved supersede link and an accepted record citing a superseded one', async () => {
    const h = await withLog({
      '0001-old-window.md': '# Old window\n\nStatus: superseded\n\nSuperseded by ADR-0002.\n',
      '0002-new-window.md':
        '# New window\n\nStatus: accepted\n\nSupersedes ADR-0001 and ADR-0009.\n',
      '0002-queue.md': '# Queue\n\nStatus: accepted\n',
      '0004-retry.md':
        '# Retry\n\nStatus: accepted\n\nThe retry waits one window, as ADR-0001 says.\n',
      '0005-cache.md': '# Cache\n\nStatus: draft\n',
    })

    expect(await checked(h)).toMatchObject({
      cites_superseded: [
        { cites: 'docs/adr/0001-old-window.md', record: 'docs/adr/0004-retry.md' },
      ],
      numbering: {
        duplicates: [
          { number: 2, records: ['docs/adr/0002-new-window.md', 'docs/adr/0002-queue.md'] },
        ],
        gaps: [3],
        unnumbered: [],
      },
      status: 'failed',
      unknown_statuses: [{ record: 'docs/adr/0005-cache.md', status: 'draft' }],
      unresolved_links: [
        { kind: 'supersede', record: 'docs/adr/0002-new-window.md', target: 'ADR-0009' },
      ],
    })
  })
})

const RECORD = '# A record\n\nWe decided.\n'

describe('the numbering check-adrs reports', () => {
  it('fails on a gap between the lowest and the highest number', async () => {
    const h = await withLog({ '0001-a.md': RECORD, '0002-b.md': RECORD, '0005-c.md': RECORD })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.numbering.gaps).toEqual([3, 4])
  })

  it('fails on two records with one number, naming both', async () => {
    const h = await withLog({ '0001-a.md': RECORD, '0002-b.md': RECORD, '0002-c.md': RECORD })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.numbering).toEqual({
      duplicates: [{ number: 2, records: ['docs/adr/0002-b.md', 'docs/adr/0002-c.md'] }],
      gaps: [],
      unnumbered: [],
    })
  })

  it('counts from the lowest number the log uses, so a log from 0000 has no gap', async () => {
    const h = await withLog({ '0000-template.md': RECORD, '0001-a.md': RECORD })

    expect((await checked(h)).numbering.gaps).toEqual([])
  })

  it('reads a file named by date as unnumbered, never as a number', async () => {
    const h = await withLog({ '2024-01-15-cache.md': RECORD, '20240302-queue.md': RECORD })

    const result = await checked(h)

    expect(result.status).toBe('passed')
    expect(result.numbering.unnumbered).toEqual([
      'docs/adr/2024-01-15-cache.md',
      'docs/adr/20240302-queue.md',
    ])
  })

  it('reads a file named by a bare date as unnumbered', async () => {
    const h = await withLog({ '0001-a.md': RECORD, '2024-01-15.md': RECORD })

    const result = await checked(h)

    expect(result.numbering.unnumbered).toEqual(['docs/adr/2024-01-15.md'])
    expect(result.numbering.gaps).toEqual([])
  })

  it('reads a file named by a date with one-digit parts as unnumbered', async () => {
    const h = await withLog({
      '0001-a.md': RECORD,
      '0002-b.md': RECORD,
      '2024-1-15-cache.md': RECORD,
    })

    const result = await checked(h)

    expect(result.numbering.unnumbered).toEqual(['docs/adr/2024-1-15-cache.md'])
    expect(result.numbering.gaps).toEqual([])
  })

  it('lists a file with no number without failing', async () => {
    const h = await withLog({
      '0001-a.md': RECORD,
      'README.md': '# Decision log\n\nOne record per file.\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('passed')
    expect(result.numbering.unnumbered).toEqual(['docs/adr/README.md'])
    expect(result.records[1]).toEqual({
      in_force: true,
      number: null,
      path: 'docs/adr/README.md',
      status: null,
      title: 'Decision log',
    })
  })
})

describe('the status values check-adrs reports', () => {
  it('fails on a status outside proposed, accepted, rejected, deprecated and superseded', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nStatus: Accepted\n',
      '0002-b.md': '---\nstatus: proposed\n---\n# B\n',
      '0003-c.md': '# C\n\n## Status\n\nRejected\n',
      '0004-d.md': '# D\n\n**Status:** deprecated\n',
      '0005-e.md': '# E\n\nStatus: Superseded by ADR-0006\n',
      '0006-f.md': '# F\n\nStatus: Pending\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.unknown_statuses).toEqual([{ record: 'docs/adr/0006-f.md', status: 'pending' }])
    expect(result.records.map((record) => [record.status, record.in_force])).toEqual([
      ['accepted', true],
      ['proposed', false],
      ['rejected', false],
      ['deprecated', false],
      ['superseded', false],
      ['pending', true],
    ])
  })
})

describe('the link targets check-adrs resolves', () => {
  it('fails on a supersede link to a record the log does not hold, by number or by file', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nStatus: superseded\n\nSuperseded by [ADR-0002](0002-b.md).\n',
      '0002-b.md':
        '# B\n\nSupersedes ADR-0001 and ADR-0007.\n\nAmends [the old rule](0009-old-rule.md).\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.unresolved_links).toEqual([
      { kind: 'supersede', record: 'docs/adr/0002-b.md', target: 'ADR-0007' },
      { kind: 'supersede', record: 'docs/adr/0002-b.md', target: '0009-old-rule.md' },
    ])
  })

  it('fails on a link or an ADR number in the text that resolves to nothing', async () => {
    const h = await withLog({
      '0001-a.md': [
        '# A',
        '',
        'See [the glossary](../../CONTEXT.md#critique-pipeline), [the old notes](../notes.md), [upstream](https://example.com/adr) and [below](#consequences).',
        '',
        'This follows ADR-0001 and ADR 0004.',
        '',
        '```',
        'ADR-0099 and [a fenced link](nowhere.md) are not cited.',
        '```',
        '',
        '[ref]: ../../missing.md',
        '',
      ].join('\n'),
    })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.unresolved_links).toEqual([
      { kind: 'link', record: 'docs/adr/0001-a.md', target: '../notes.md' },
      { kind: 'link', record: 'docs/adr/0001-a.md', target: 'ADR 0004' },
      { kind: 'link', record: 'docs/adr/0001-a.md', target: '../../missing.md' },
    ])
  })

  it('never reads an ADR number inside a bare URL as a reference', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nSee https://example.com/adr/ADR-0004.md for the upstream record.\n',
    })

    const result = await checked(h)

    expect(result.unresolved_links).toEqual([])
    expect(result.status).toBe('passed')
  })

  it('reads a footnote as a note, never as a link', async () => {
    const h = await withLog({
      '0001-a.md':
        '# A\n\nWe follow Nygard.[^1]\n\n[^1]: Nygard, Documenting Architecture Decisions, 2011.\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('passed')
    expect(result.unresolved_links).toEqual([])
  })

  it('never resolves a link outside the repository', async () => {
    const h = await withLog({ '0001-a.md': '# A\n\nSee [outside](../../../elsewhere.md).\n' })

    expect((await checked(h)).unresolved_links).toEqual([
      { kind: 'link', record: 'docs/adr/0001-a.md', target: '../../../elsewhere.md' },
    ])
  })
})

describe('the superseded records check-adrs finds still cited', () => {
  it('fails on a record in force that cites a superseded one, by number or by file', async () => {
    const h = await withLog({
      '0001-old-window.md': '# Old window\n\nStatus: superseded\n\nSuperseded by ADR-0002.\n',
      '0002-new-window.md': '# New window\n\nSupersedes ADR-0001.\n',
      '0003-retry.md':
        '# Retry\n\nThe retry waits one window, as ADR-0001 says, and [the old record](0001-old-window.md) explains.\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('failed')
    expect(result.cites_superseded).toEqual([
      { cites: 'docs/adr/0001-old-window.md', record: 'docs/adr/0003-retry.md' },
    ])
  })

  it('counts a record another in force replaces as superseded, even with no status', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nWe decided.\n',
      '0002-b.md': '# B\n\nReplaces ADR-0001.\n',
      '0003-c.md': '# C\n\nBuilds on ADR-0001.\n',
    })

    expect((await checked(h)).cites_superseded).toEqual([
      { cites: 'docs/adr/0001-a.md', record: 'docs/adr/0003-c.md' },
    ])
  })

  it('counts a record another in force replaces as out of force, so its own citations are not reported', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nWe decided, per ADR-0003.\n',
      '0002-b.md': '# B\n\nReplaces ADR-0001.\n',
      '0003-c.md': '# C\n\nStatus: superseded\n',
    })

    const result = await checked(h)

    expect(result.cites_superseded).toEqual([])
    expect(result.records[0]).toMatchObject({
      in_force: false,
      path: 'docs/adr/0001-a.md',
      status: null,
    })
  })

  it('lets a record that is not in force cite a superseded one', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nStatus: superseded\n',
      '0002-b.md': '# B\n\nStatus: deprecated\n\nBuilt on ADR-0001.\n',
    })

    expect((await checked(h)).cites_superseded).toEqual([])
  })
})

describe('the root-relative and unnumbered files check-adrs reads', () => {
  it('resolves a link that starts with a slash from the repository root', async () => {
    const h = await withLog({
      '0001-nightly-job.md':
        '# The nightly job frobnicates widgets\n\nSee [the window](/docs/adr/0002-window.md).\n',
      '0002-window.md': '# Widgets keep a window\n\nStatus: accepted\n',
    })

    expect((await checked(h)).status).toBe('passed')
  })

  it('lets an unnumbered file such as an index list a superseded record without failing the log', async () => {
    const h = await withLog({
      '0001-old.md': '# Old\n\nStatus: superseded\n',
      '0002-new.md': '# New\n\nStatus: accepted\n\nSupersedes [ADR-0001](0001-old.md).\n',
      'README.md': '# Index\n\n- [Old](0001-old.md)\n- [New](0002-new.md)\n',
    })

    expect((await checked(h)).cites_superseded).toEqual([])
  })
})

describe('the clauses of a supersede line', () => {
  it('lets a link beside a Supersedes clause stay in force and be cited', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nStatus: accepted\n',
      '0002-b.md': '# B\n\nStatus: accepted\n',
      '0003-c.md': '# C\n\nSupersedes [B](0002-b.md); builds on [A](0001-a.md).\n',
      '0004-d.md': '# D\n\nDepends on [A](0001-a.md).\n',
    })

    const result = await checked(h)

    expect(result.status).toBe('passed')
    expect(result.records.map((record) => [record.number, record.in_force])).toEqual([
      [1, true],
      [2, false],
      [3, true],
      [4, true],
    ])
  })

  it('counts a record that says it was superseded as superseded, whatever its status', async () => {
    const h = await withLog({
      '0001-a.md': '# A\n\nStatus: accepted\n\nSuperseded by [ADR-0002](0002-b.md).\n',
      '0002-b.md': '# B\n\nStatus: accepted\n',
      '0003-c.md': '# C\n\nDepends on [A](0001-a.md).\n',
    })

    const result = await checked(h)

    expect(result.records[0]?.in_force).toBe(false)
    expect(result.cites_superseded).toEqual([
      { cites: 'docs/adr/0001-a.md', record: 'docs/adr/0003-c.md' },
    ])
    expect(result.status).toBe('failed')
  })
})

describe("check-adrs on agent-kit's own decision log", () => {
  it('passes with no structural error, every record counting as in force', async () => {
    const kit = fileURLToPath(new URL('../../../../../', import.meta.url))
    const h = await withLog({})
    await cp(join(kit, 'docs'), join(h.repo, 'docs'), { recursive: true })
    await cp(join(kit, 'CONTEXT.md'), join(h.repo, 'CONTEXT.md'))
    const names = (await readdir(join(kit, ADR_DIRECTORY))).filter((name) => name.endsWith('.md'))

    const result = await checked(h)

    expect(result).toMatchObject({
      cites_superseded: [],
      numbering: { duplicates: [], gaps: [], unnumbered: [] },
      status: 'passed',
      unknown_statuses: [],
      unresolved_links: [],
    })
    expect(result.records.map((record) => record.path)).toEqual(
      names.map((name) => join(ADR_DIRECTORY, name)),
    )
    expect(result.records.every((record) => record.in_force && record.status === null)).toBe(true)
  })
})

describe('check-adrs refusals', () => {
  it('is refused when the repository has no decision records', async () => {
    const h = await withLog({})

    expect(await check(h)).toEqual({
      message: 'found no decision records in docs/adr/ to review',
      reason: 'no-adrs',
      status: 'refused',
    })
  })

  it('is refused for a repository whose remotes refuse a critique, with no allowlist entry needed otherwise', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await check(h)).toMatchObject({ reason: 'no-remote', status: 'refused' })
  })
})
