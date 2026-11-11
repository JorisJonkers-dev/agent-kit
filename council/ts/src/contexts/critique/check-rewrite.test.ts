import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../../test/critique/harness.js'
import type { CheckRewriteResult, Refusal } from './index.js'

/**
 * The rewrite check (#228): after `/critique-adrs` applies its editorial
 * edits, it confirms that every decision sentence quoted from the frozen
 * packet appears byte-identical in exactly one rewritten file, that no
 * rewritten file labels a decision sentence that is not an original one, and
 * that a split marks the original record superseded by the split files. A
 * failure is what sends the record back to the owner as a new-decision
 * question.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function withAdrs(records: Readonly<Record<string, string>>): Promise<Harness> {
  const h = await harness()
  open.push(h)
  for (const [name, text] of Object.entries(records)) await h.writeText(join(h.repo, 'docs', 'adr', name), text)
  return h
}

interface RewriteInput {
  readonly original_sentences: readonly string[]
  readonly rewritten: readonly { readonly path: string; readonly decision_sentences: readonly string[] }[]
  readonly split?: { readonly original: string; readonly files: readonly string[] }
}

async function check(h: Harness, input: RewriteInput): Promise<CheckRewriteResult | Refusal> {
  return h.commands.checkRewrite.run({ repo: h.repo, ...input })
}

async function checked(h: Harness, input: RewriteInput): Promise<CheckRewriteResult> {
  const result = await check(h, input)
  if (result.status === 'refused') throw new Error(`check-rewrite refused: ${result.message}`)
  return result
}

const SENTENCE = 'We frobnicate widgets nightly.'

describe('check-rewrite on a rewrite that keeps its decision sentence', () => {
  it('passes when the sentence is byte-identical in exactly one rewritten file', async () => {
    const h = await withAdrs({ '0001-nightly.md': `# Nightly\n\nStatus: accepted\n\n${SENTENCE}\n` })

    expect(await checked(h, { original_sentences: [SENTENCE], rewritten: [{ decision_sentences: [SENTENCE], path: 'docs/adr/0001-nightly.md' }] })).toEqual({
      failures: [],
      status: 'passed',
    })
  })

  it('passes a rewrite with no split, which never needs a supersede mark', async () => {
    const h = await withAdrs({ '0001-nightly.md': `# Nightly\n\n${SENTENCE}\n` })

    expect((await checked(h, { original_sentences: [SENTENCE], rewritten: [{ decision_sentences: [SENTENCE], path: 'docs/adr/0001-nightly.md' }] })).status).toBe('passed')
  })
})

describe('check-rewrite on a rewrite that changed one word of a decision sentence', () => {
  it('fails: the original sentence is in no file, and the changed one is not an original', async () => {
    const h = await withAdrs({ '0001-nightly.md': '# Nightly\n\nWe frobnicate widgets nightly!\n' })

    expect(
      await checked(h, {
        original_sentences: [SENTENCE],
        rewritten: [{ decision_sentences: ['We frobnicate widgets nightly!'], path: 'docs/adr/0001-nightly.md' }],
      }),
    ).toEqual({
      failures: [
        { check: 'sentence-preserved', found_in: [], problem: 'missing', sentence: SENTENCE },
        { check: 'no-new-decision-sentence', file: 'docs/adr/0001-nightly.md', sentence: 'We frobnicate widgets nightly!' },
      ],
      status: 'failed',
    })
  })
})

describe('check-rewrite on a decision sentence in two rewritten files', () => {
  it('fails with the sentence marked duplicated', async () => {
    const h = await withAdrs({
      '0001-window.md': `# Window\n\n${SENTENCE}\n`,
      '0002-split.md': `# Split\n\n${SENTENCE}\n`,
    })

    expect(
      await checked(h, {
        original_sentences: [SENTENCE],
        rewritten: [
          { decision_sentences: [SENTENCE], path: 'docs/adr/0001-window.md' },
          { decision_sentences: [SENTENCE], path: 'docs/adr/0002-split.md' },
        ],
      }),
    ).toEqual({
      failures: [
        {
          check: 'sentence-preserved',
          found_in: ['docs/adr/0001-window.md', 'docs/adr/0002-split.md'],
          problem: 'duplicated',
          sentence: SENTENCE,
        },
      ],
      status: 'failed',
    })
  })
})

describe('check-rewrite on a split Mega-ADR', () => {
  const NIGHTLY = 'We frobnicate widgets nightly.'
  const WINDOW = 'We keep a seven second canary window.'
  const rewritten = [
    { decision_sentences: [], path: 'docs/adr/0001-mega.md' },
    { decision_sentences: [NIGHTLY], path: 'docs/adr/0002-first.md' },
    { decision_sentences: [WINDOW], path: 'docs/adr/0003-second.md' },
  ]
  const split = { files: ['docs/adr/0002-first.md', 'docs/adr/0003-second.md'], original: 'docs/adr/0001-mega.md' }

  async function splitAdrs(original: string): Promise<Harness> {
    return withAdrs({
      '0001-mega.md': original,
      '0002-first.md': `# First\n\n${NIGHTLY}\n`,
      '0003-second.md': `# Second\n\n${WINDOW}\n`,
    })
  }

  it('passes when the original is marked superseded by every split file, by link', async () => {
    const h = await splitAdrs('# Mega\n\nStatus: superseded\n\nSuperseded by [the first record](0002-first.md) and [the second record](0003-second.md).\n')

    expect(await checked(h, { original_sentences: [NIGHTLY, WINDOW], rewritten, split })).toEqual({ failures: [], status: 'passed' })
  })

  it('passes when the original names each split file by its ADR number', async () => {
    const h = await splitAdrs('# Mega\n\nSuperseded by ADR-0002 and ADR-0003.\n')

    expect((await checked(h, { original_sentences: [NIGHTLY, WINDOW], rewritten, split })).status).toBe('passed')
  })

  it('fails when the split leaves no supersede mark on the original', async () => {
    const h = await splitAdrs('# Mega\n\nStatus: superseded\n')

    expect(await checked(h, { original_sentences: [NIGHTLY, WINDOW], rewritten, split })).toEqual({
      failures: [{ check: 'split-superseded', original: 'docs/adr/0001-mega.md', unmarked: ['docs/adr/0002-first.md', 'docs/adr/0003-second.md'] }],
      status: 'failed',
    })
  })

  it('fails naming only the split files the original does not mark', async () => {
    const h = await splitAdrs('# Mega\n\nSuperseded by [ADR-0002](0002-first.md).\n')

    expect((await checked(h, { original_sentences: [NIGHTLY, WINDOW], rewritten, split })).failures).toEqual([
      { check: 'split-superseded', original: 'docs/adr/0001-mega.md', unmarked: ['docs/adr/0003-second.md'] },
    ])
  })
})

describe('check-rewrite on a rewritten file that labels a decision sentence that is not an original', () => {
  it('fails naming the new sentence', async () => {
    const h = await withAdrs({ '0001-nightly.md': `# Nightly\n\n${SENTENCE}\n\nWe retry on Sundays.\n` })

    expect(
      await checked(h, {
        original_sentences: [SENTENCE],
        rewritten: [{ decision_sentences: [SENTENCE, 'We retry on Sundays.'], path: 'docs/adr/0001-nightly.md' }],
      }),
    ).toEqual({
      failures: [{ check: 'no-new-decision-sentence', file: 'docs/adr/0001-nightly.md', sentence: 'We retry on Sundays.' }],
      status: 'failed',
    })
  })
})

describe('check-rewrite refusals and errors', () => {
  it('is refused for a repository whose remotes refuse a critique', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await check(h, { original_sentences: [SENTENCE], rewritten: [{ decision_sentences: [], path: 'docs/adr/0001-a.md' }] })).toMatchObject({
      reason: 'no-remote',
      status: 'refused',
    })
  })

  it('rejects a rewritten file that is not in the working tree', async () => {
    const h = await withAdrs({})

    await expect(checked(h, { original_sentences: [], rewritten: [{ decision_sentences: [], path: 'docs/adr/0001-missing.md' }] })).rejects.toThrow(
      /docs\/adr\/0001-missing\.md does not exist/,
    )
  })

  it('rejects input without rewritten files', async () => {
    const h = await withAdrs({})

    await expect(h.commands.checkRewrite.run({ repo: h.repo })).rejects.toThrow(/check-rewrite input/)
  })
})

describe('check-rewrite on malformed or hostile input', () => {
  it('fails a decision sentence that appears twice in one file', async () => {
    const h = await withAdrs({ '0001-window.md': `# Window\n\n${SENTENCE}\n\n${SENTENCE}\n` })

    expect(
      await checked(h, { original_sentences: [SENTENCE], rewritten: [{ decision_sentences: [SENTENCE], path: 'docs/adr/0001-window.md' }] }),
    ).toMatchObject({
      failures: [{ check: 'sentence-preserved', found_in: ['docs/adr/0001-window.md'], problem: 'duplicated' }],
      status: 'failed',
    })
  })

  it('fails every original sentence when no file was rewritten', async () => {
    const h = await withAdrs({})

    expect(await checked(h, { original_sentences: [SENTENCE], rewritten: [] })).toMatchObject({
      failures: [{ problem: 'missing' }],
      status: 'failed',
    })
  })

  it('rejects an empty decision sentence', async () => {
    const h = await withAdrs({ '0001-window.md': `# Window\n\n${SENTENCE}\n` })

    await expect(check(h, { original_sentences: [''], rewritten: [{ decision_sentences: [], path: 'docs/adr/0001-window.md' }] })).rejects.toThrow(
      /empty/,
    )
  })

  it('rejects a rewritten path that climbs out of the repository', async () => {
    const h = await withAdrs({})

    await expect(check(h, { original_sentences: [SENTENCE], rewritten: [{ decision_sentences: [], path: '../outside.md' }] })).rejects.toThrow(
      /outside the repository/,
    )
  })
})
