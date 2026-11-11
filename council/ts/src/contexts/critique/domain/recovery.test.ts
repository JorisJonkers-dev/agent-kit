import { describe, expect, it } from 'vitest'

import { citedIssues, HISTORY_CAP_BYTES, HISTORY_MAX_ISSUES, isSourcePath, renderHistoryLog, sourceLines, sourceSample, type HistoryEntry } from './recovery.js'

function entry(overrides: Partial<HistoryEntry> = {}): HistoryEntry {
  return { body: '', commit: 'abc', date: '2026-02-01T00:00:00+00:00', files: ['src/a.ts'], subject: 'Subject', ...overrides }
}

describe('isSourcePath', () => {
  it('takes code by its extension and leaves documents, data and dotfiles out', () => {
    expect(['src/a.ts', 'lib/b.PY', 'x/main.rs', 'a.nix'].map(isSourcePath)).toEqual([true, true, true, true])
    expect(['README.md', 'package.json', '.ts', 'Makefile', 'dir.ts/readme'].map(isSourcePath)).toEqual([false, false, false, false, false])
  })
})

describe('sourceLines and sourceSample', () => {
  const tracked = [
    { path: 'src/deep/a.ts', text: 'one\ntwo\n' },
    { path: 'main.ts', text: 'one\n' },
    { path: 'docs/guide.md', text: 'a\nb\nc\n' },
    { path: 'bin/blob.rs', text: null },
  ]

  it('counts the lines of source files that are text, and no others', () => {
    expect(sourceLines(tracked)).toBe(3)
  })

  it('samples source shallowest first, then by path', () => {
    expect(sourceSample(tracked)).toEqual(['main.ts', 'src/deep/a.ts'])
    expect(sourceSample([{ path: 'b.ts', text: 'x' }, { path: 'a.ts', text: 'x' }])).toEqual(['a.ts', 'b.ts'])
  })
})

describe('renderHistoryLog', () => {
  it('writes each commit with its date, files, subject and body', () => {
    const { left_out: left, text } = renderHistoryLog([entry({ body: 'Why.' }), entry({ commit: 'def' })], 'c9')

    expect(left).toBe(0)
    expect(text).toContain('reachable from `c9`')
    expect(text).toContain('## abc 2026-02-01T00:00:00+00:00\nFiles: src/a.ts\n\nSubject\n\nWhy.\n')
    expect(text).toContain('## def 2026-02-01T00:00:00+00:00\nFiles: src/a.ts\n\nSubject\n\n')
  })

  it('stops at the cap and says how many older commits did not fit', () => {
    const long = 'x'.repeat(HISTORY_CAP_BYTES / 4)
    const { left_out: left, text } = renderHistoryLog(Array.from({ length: 6 }, (_, n) => entry({ body: long, commit: `c${String(n)}` })), 'c9')

    expect(left).toBe(3)
    expect(Buffer.byteLength(text)).toBeLessThanOrEqual(HISTORY_CAP_BYTES + 200)
    expect(text.endsWith(`3 older commits did not fit the history cap of ${String(HISTORY_CAP_BYTES)} bytes.\n`)).toBe(true)
  })
})

describe('citedIssues', () => {
  it('lists each issue number once, in the order cited, from subjects and bodies', () => {
    const entries = [entry({ body: 'Refs #9 and #4', subject: 'Fix (#4)' }), entry({ subject: 'Again #9, then #11' })]

    expect(citedIssues(entries)).toEqual([4, 9, 11])
  })

  it('ignores a hash inside a word, a path or a heading, and caps the count', () => {
    expect(citedIssues([entry({ body: 'a#5 x/#6 ##7' })])).toEqual([])
    const many = entry({ body: Array.from({ length: 30 }, (_, n) => `#${String(n + 1)}`).join(' ') })
    expect(citedIssues([many])).toHaveLength(HISTORY_MAX_ISSUES)
  })
})
