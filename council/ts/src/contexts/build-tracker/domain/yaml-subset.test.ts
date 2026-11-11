import { describe, expect, it } from 'vitest'

import { BuildTrackerError } from './state.js'
import { isYamlObject, parseYamlSubset } from './yaml-subset.js'

/** A source of `lines`, one per line. */
function yaml(...lines: readonly string[]): string {
  return lines.join('\n')
}

/** The error the reader refuses `source` with. */
function refusal(source: string): BuildTrackerError {
  try {
    parseYamlSubset(source)
  } catch (error) {
    if (error instanceof BuildTrackerError) return error
    throw error
  }
  throw new Error(`the reader accepted ${JSON.stringify(source)}`)
}

function expectRefused(source: string): void {
  expect(() => parseYamlSubset(source)).toThrow(BuildTrackerError)
  expect(refusal(source).code).toBe('invalid-tickets')
}

describe('parseYamlSubset', () => {
  it('reads a mapping of plain, integer, boolean, null and empty-list scalars', () => {
    const source = yaml(
      'name: widgets',
      'count: 3',
      'negative: -2',
      'enabled: true',
      'disabled: false',
      'nothing: null',
      'tilde: ~',
      'empty: []',
    )

    expect(parseYamlSubset(source)).toEqual({
      count: 3,
      disabled: false,
      empty: [],
      enabled: true,
      name: 'widgets',
      negative: -2,
      nothing: null,
      tilde: null,
    })
  })

  it('reads a nested mapping', () => {
    expect(parseYamlSubset(yaml('outer:', '  inner: 1', '  deeper:', '    leaf: x'))).toEqual({
      outer: { deeper: { leaf: 'x' }, inner: 1 },
    })
  })

  it('reads a nested mapping that ends before its parent does', () => {
    expect(parseYamlSubset(yaml('outer:', '  inner: 1', 'other: 2'))).toEqual({
      outer: { inner: 1 },
      other: 2,
    })
  })

  it('reads a block list level with its key', () => {
    expect(parseYamlSubset(yaml('items:', '- a', '- b'))).toEqual({ items: ['a', 'b'] })
  })

  it('reads a block list indented under its key', () => {
    expect(parseYamlSubset(yaml('items:', '  - a', '  - b'))).toEqual({ items: ['a', 'b'] })
  })

  it('reads a list of mappings', () => {
    expect(
      parseYamlSubset(yaml('tickets:', '  - key: a', '    issue: 1', '  - key: b', '    issue: 2')),
    ).toEqual({
      tickets: [
        { issue: 1, key: 'a' },
        { issue: 2, key: 'b' },
      ],
    })
  })

  it('reads a bare list item with a nested block under it', () => {
    expect(parseYamlSubset(yaml('items:', '  -', '    key: a'))).toEqual({ items: [{ key: 'a' }] })
  })

  it('reads single-quoted and double-quoted scalars', () => {
    expect(parseYamlSubset(yaml("single: 'it''s'", 'double: "a \\"b\\""'))).toEqual({
      double: 'a "b"',
      single: "it's",
    })
  })

  it('reads a key with no value as null, before a sibling or at the end of the file', () => {
    expect(parseYamlSubset(yaml('a:', 'b: 1'))).toEqual({ a: null, b: 1 })
    expect(parseYamlSubset(yaml('a: 1', 'b:'))).toEqual({ a: 1, b: null })
  })

  it('skips comments and blank lines', () => {
    expect(
      parseYamlSubset(yaml('# a comment', '', 'a: 1', '   # an indented comment', 'b: 2')),
    ).toEqual({ a: 1, b: 2 })
  })

  it('strips a trailing comment from a plain scalar', () => {
    expect(parseYamlSubset(yaml('a: value # note'))).toEqual({ a: 'value' })
  })

  it('reads CRLF input', () => {
    expect(parseYamlSubset('a: 1\r\nb: 2\r\n')).toEqual({ a: 1, b: 2 })
  })
})

describe('parseYamlSubset refusals', () => {
  it('refuses an empty file', () => {
    expectRefused('')
    expectRefused('\n\n# only a comment\n')
  })

  it('refuses a tab indent', () => {
    expectRefused('\ta: 1')
    expectRefused('  \tb: 2')
  })

  it('refuses a single-quoted scalar that does not end with its quote', () => {
    expectRefused("a: 'x")
  })

  it('refuses a double-quoted value that is not a JSON string', () => {
    expectRefused('a: "x')
  })

  it('refuses the YAML forms it does not read', () => {
    expectRefused('a: [1, 2]')
    expectRefused('a: |')
  })

  it('refuses a plain value holding `: `', () => {
    expectRefused('a: b: c')
  })

  it('refuses a duplicate key', () => {
    expectRefused(yaml('a: 1', 'a: 2'))
  })

  it('refuses a line that is not `key: value`', () => {
    expectRefused('just text')
  })

  it('refuses a deeper indent after a scalar', () => {
    expectRefused(yaml('a: 1', '  b: 2'))
  })

  it('refuses a list item with no value', () => {
    expectRefused(yaml('items:', '  -', '  - b'))
    expectRefused(yaml('items:', '  -'))
  })

  it('refuses a line that does not fit its parent', () => {
    expectRefused(yaml('a: 1', '- b'))
    expectRefused(yaml('- a', 'b: 1'))
    expectRefused(yaml('  a: 1', 'b: 2'))
  })
})

describe('isYamlObject', () => {
  it('accepts a mapping and nothing else', () => {
    expect(isYamlObject({ a: 1 })).toBe(true)
    expect(isYamlObject({})).toBe(true)
    expect(isYamlObject([])).toBe(false)
    expect(isYamlObject(null)).toBe(false)
    expect(isYamlObject('a')).toBe(false)
    expect(isYamlObject(1)).toBe(false)
    expect(isYamlObject(undefined)).toBe(false)
  })
})
