import { describe, expect, it } from 'vitest'

import { EMPTY_ALLOWLIST, parseAllowlist } from './allowlist.js'

describe('parseAllowlist', () => {
  it('reads an entry families, code and history', () => {
    const allowlist = parseAllowlist({
      repositories: {
        'github.com/acme/widgets': { code: true, families: ['anthropic', 'openai'], history: false, updated_at: 'x' },
      },
      version: 1,
    })

    expect(allowlist.repositories['github.com/acme/widgets']).toEqual({
      code: true,
      families: ['anthropic', 'openai'],
      history: false,
    })
  })

  it('reads an entry recorded before a permission existed as allowing neither', () => {
    const allowlist = parseAllowlist({
      repositories: { 'github.com/acme/widgets': { families: ['mistral'], updated_at: 'x' } },
      version: 1,
    })

    expect(allowlist.repositories['github.com/acme/widgets']).toEqual({ code: false, families: ['mistral'], history: false })
  })

  it('reads a file with no repositories as an empty allowlist', () => {
    expect(parseAllowlist({ repositories: {}, version: 1 })).toEqual(EMPTY_ALLOWLIST)
  })

  it.each<[string, unknown]>([
    ['not an object', []],
    ['a version it does not know', { repositories: {}, version: 2 }],
    ['repositories that are not an object', { repositories: [], version: 1 }],
    ['an entry that is not an object', { repositories: { 'github.com/acme/widgets': 'mistral' }, version: 1 }],
    ['families written as one string', { repositories: { 'github.com/acme/widgets': { families: 'mistral' } }, version: 1 }],
    ['a family that is not a string', { repositories: { 'github.com/acme/widgets': { families: [7] } }, version: 1 }],
    ['a code permission that is not true or false', { repositories: { 'github.com/acme/widgets': { code: 'yes', families: [] } }, version: 1 }],
    ['a history permission that is not true or false', { repositories: { 'github.com/acme/widgets': { families: [], history: 1 } }, version: 1 }],
  ])('refuses %s', (_name, raw) => {
    expect(() => parseAllowlist(raw)).toThrow(/invalid|must be/)
  })
})
