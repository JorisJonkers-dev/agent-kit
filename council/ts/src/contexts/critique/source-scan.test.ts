import { describe, expect, it } from 'vitest'

import { changedLines, contextSlices } from './domain/context-slices.js'
import { buildRepoMap, renderCodeNavMap, renderRepoMap, SourceGraph } from './domain/repo-map.js'
import { mentionLines, resolveImport, scanSource } from './domain/source-scan.js'

/**
 * The symbol and import scan behind a code packet's repo map and context
 * slices (#254), at the scanner itself: the languages it reads, what it
 * reads from each, and the line diff that finds a change's lines. #233
 * extends the scan to Kotlin, Go and Rust.
 */

const TYPESCRIPT = [
  "import { a } from './a.js'",
  "import type { B } from '../lib/b'",
  "import './side-effect.js'",
  "export * from './again'",
  "const lazy = import('./lazy.mjs')",
  "const legacy = require('./legacy')",
  "import { z } from 'zod'",
  '',
  '/** A widget. */',
  'export class Widget<T extends { id: number }> {',
  '  constructor(readonly value: T) {}',
  '}',
  '',
  'export interface Shape {',
  '  readonly sides: number',
  '}',
  'export type Id = string | number',
  'export enum Colour {',
  '  Red,',
  '}',
  'export const enum Flag {',
  '  On,',
  '}',
  'export const LIMIT: number = 3',
  'export const double = (n: number): number => n * 2',
  'export const asyncDouble = async (n: number) => {',
  '  return n * 2',
  '}',
  'export let counter = 0',
  'var legacyName = function () {}',
  'export async function* stream(',
  '  first: number,',
  '  second: number,',
  '): AsyncGenerator<number> {',
  '  yield first + second',
  '}',
  'export default function () {}',
  'export const generic = <T,>(value: T): T => value',
  'export const label = `x`',
  'export declare function overload(a: number): void',
  '',
].join('\n')

describe('the TypeScript and JavaScript scan', () => {
  const scan = scanSource('src/widget.ts', TYPESCRIPT)

  it('reads every top-level declaration with its kind, its lines and its signature', () => {
    expect(scan?.symbols.map(({ first, kind, last, name, signature }) => [name, kind, first, last, signature])).toEqual([
      ['lazy', 'const', 5, 5, 'const lazy'],
      ['legacy', 'const', 6, 6, 'const legacy'],
      ['Widget', 'class', 10, 12, 'export class Widget<T extends { id: number }>'],
      ['Shape', 'interface', 14, 16, 'export interface Shape'],
      ['Id', 'type', 17, 17, 'export type Id = string | number'],
      ['Colour', 'enum', 18, 20, 'export enum Colour'],
      ['Flag', 'const enum', 21, 23, 'export const enum Flag'],
      ['LIMIT', 'const', 24, 24, 'export const LIMIT: number'],
      ['double', 'const', 25, 25, 'export const double = (n: number): number =>'],
      ['asyncDouble', 'const', 26, 28, 'export const asyncDouble = async (n: number) =>'],
      ['counter', 'let', 29, 29, 'export let counter'],
      ['legacyName', 'var', 30, 30, 'var legacyName'],
      ['stream', 'function*', 31, 36, 'export async function* stream( first: number, second: number, ): AsyncGenerator<number>'],
      ['generic', 'const', 38, 38, 'export const generic = <T,>(value: T): T =>'],
      ['label', 'const', 39, 39, 'export const label'],
      ['overload', 'function', 40, 40, 'export declare function overload(a: number): void'],
    ])
  })

  it('reads every import, side-effect, dynamic and require form', () => {
    expect(scan?.imports).toEqual(['./a.js', '../lib/b', './side-effect.js', './again', './lazy.mjs', './legacy', 'zod'])
  })

  it('resolves a relative import to a tracked file, the way TypeScript maps .js to .ts', () => {
    const tracked = new Set(['src/a.ts', 'lib/b.tsx', 'src/side-effect.js', 'src/again/index.ts', 'src/lazy.mts', 'src/legacy.cjs'])

    expect(scan?.imports.map((specifier) => resolveImport('src/widget.ts', specifier, tracked))).toEqual([
      'src/a.ts',
      'lib/b.tsx',
      'src/side-effect.js',
      'src/again/index.ts',
      'src/lazy.mts',
      'src/legacy.cjs',
      undefined,
    ])
  })

  it('shortens a signature past 200 characters', () => {
    const long = scanSource('a.ts', `export function f(${'a: number, '.repeat(30)}) {\n}\n`)?.symbols[0]?.signature ?? ''

    expect(long).toHaveLength(200)
    expect(long.endsWith('…')).toBe(true)
  })
})

const PYTHON = [
  'from . import sibling',
  'from ..pkg.mod import name, other as alias',
  'from pkg import (',
  '    first,',
  ')',
  'import os, pkg.deep',
  '',
  'LIMIT: int = 3',
  'counter = 0',
  '',
  '@decorator',
  'async def fetch(',
  '    url: str,',
  ') -> bytes:',
  '    return b""',
  '',
  'def plain(a, b=1):',
  '    return a == b',
  '',
  'class Widget(Base):',
  '    def method(self):',
  '        pass',
  '',
].join('\n')

describe('the Python scan', () => {
  const scan = scanSource('app/pkg/widget.py', PYTHON)

  it('reads top-level functions, classes and assignments with their signatures', () => {
    expect(scan?.symbols.map(({ first, kind, last, name, signature }) => [name, kind, first, last, signature])).toEqual([
      ['LIMIT', 'assignment', 8, 8, 'LIMIT: int'],
      ['counter', 'assignment', 9, 9, 'counter'],
      ['fetch', 'async def', 12, 15, 'async def fetch( url: str, ) -> bytes'],
      ['plain', 'def', 17, 18, 'def plain(a, b=1)'],
      ['Widget', 'class', 20, 22, 'class Widget(Base)'],
    ])
  })

  it('reads each module a from-import or an import names', () => {
    expect(scan?.imports).toEqual(['.', '.sibling', '..pkg.mod', '..pkg.mod.name', '..pkg.mod.other', 'pkg', 'pkg.first', 'os', 'pkg.deep'])
  })

  it('resolves relative imports from the package and absolute ones from the root or one nested source root', () => {
    const tracked = new Set(['app/pkg/sibling.py', 'app/pkg/mod/__init__.py', 'app/pkg/__init__.py', 'src/pkg/deep.py', 'top.py'])
    const resolve = (specifier: string, from = 'app/pkg/widget.py') => resolveImport(from, specifier, tracked)

    expect(resolve('.sibling')).toBe('app/pkg/sibling.py')
    expect(resolve('..pkg.mod')).toBe('app/pkg/mod/__init__.py')
    expect(resolve('.')).toBe('app/pkg/__init__.py')
    expect(resolve('top')).toBe('top.py')
    expect(resolve('pkg.deep')).toBe('src/pkg/deep.py')
    expect(resolve('os')).toBeUndefined()
    expect(resolve('.', 'top.py')).toBeUndefined()
  })

  it('leaves an absolute import that two source roots could mean unresolved', () => {
    expect(resolveImport('a.py', 'pkg', new Set(['src/pkg.py', 'lib/pkg.py']))).toBeUndefined()
  })
})

describe('a file in a language the scan does not read', () => {
  it('has no scan', () => {
    expect(scanSource('README.md', '# Widgets\n')).toBeUndefined()
    expect(resolveImport('README.md', './a', new Set(['a.ts']))).toBeUndefined()
  })

  it('gets the lines around each change, ten either side, as its slices', () => {
    const before = Array.from({ length: 40 }, (_, i) => `line ${String(i + 1)}`).join('\n') + '\n'
    const after = before.replace('line 20\n', 'line twenty\n')

    expect(contextSlices(SourceGraph.build([]), [{ before, path: 'notes.md', text: after }])).toEqual([
      { first: 10, last: 30, path: 'notes.md', reason: 'changed' },
    ])
  })
})

describe('the slices of a change outside every symbol of a source file', () => {
  it('are the three lines either side of it', () => {
    const before = "import { a } from './a.js'\n\nexport const b = 1\n"
    const after = "import { a } from './a2.js'\n\nexport const b = 1\n"

    expect(contextSlices(SourceGraph.build([{ path: 'b.ts', text: after }]), [{ before, path: 'b.ts', text: after }])).toEqual([
      { first: 1, last: 3, path: 'b.ts', reason: 'changed' },
    ])
  })
})

describe('the lines a change touches', () => {
  it('are every line of an added file', () => {
    expect(changedLines(undefined, 'a\nb\n')).toEqual([1, 2])
  })

  it('are none when nothing changed', () => {
    expect(changedLines('a\nb\n', 'a\nb\n')).toEqual([])
  })

  it('are the line after a removal, or the last line when the removal ends the file', () => {
    expect(changedLines('a\nb\nc\n', 'a\nc\n')).toEqual([2])
    expect(changedLines('a\nb\nc\n', 'a\nb\n')).toEqual([2])
    expect(changedLines('a\nb\n', '')).toEqual([])
  })

  it('leave out the lines a rewrite keeps', () => {
    expect(changedLines('a\nx\nb\ny\nc\n', 'a\nb\nnew\nc\n')).toEqual([3])
  })

  it('find each added line around a kept one', () => {
    expect(changedLines('a\nq\nz\n', 'a\nn\nq\nw\n')).toEqual([2, 4])
  })

  it('touch the line where lines were only removed between kept ones', () => {
    expect(changedLines('x\na\ny\n', 'a\n')).toEqual([1])
  })

  it('take the whole changed middle of a file too large to diff line by line', () => {
    const before = Array.from({ length: 2100 }, (_, i) => `old ${String(i)}`).join('\n')
    const after = Array.from({ length: 2100 }, (_, i) => `new ${String(i)}`).join('\n')

    expect(changedLines(before, after)).toHaveLength(2100)
  })
})

describe('the words a scan finds', () => {
  it('match a name with a dollar sign as a whole word only', () => {
    expect(mentionLines('const $el = 1\nconst $element = 2\nuse($el)\n', '$el')).toEqual([1, 3])
  })
})

describe('the repo map', () => {
  it('says so when no file is in a language the scan reads', () => {
    expect(renderRepoMap(buildRepoMap(SourceGraph.build([{ path: 'README.md', text: '# x\n' }])), { capBytes: 4000, commit: 'c1' }).text).toContain(
      'No source file in a language the scan reads.',
    )
  })

  it('says so when a source file has no top-level symbols', () => {
    expect(renderRepoMap(buildRepoMap(SourceGraph.build([{ path: 'run.py', text: 'main()\n' }])), { capBytes: 4000, commit: 'c1' }).text).toContain(
      '## `run.py` (referenced by 0 files)\n\nNo top-level symbols.\n',
    )
  })
})

describe('the repo map built from code-nav', () => {
  const ranking = {
    files: [
      { path: 'a.ts', refs: 3 },
      { path: 'b.ts', refs: 1 },
    ],
    symbols: [{ kind: 'fn', line: 2, name: 'a', path: 'a.ts', refs: 3 }],
  }

  it('lists every ranked file with its symbols', () => {
    const map = renderCodeNavMap(ranking, { capBytes: 4000, commit: 'c1' })

    expect(map).toMatchObject({ dropped: 0, listed: 2, source: 'code-nav' })
    expect(map.text).toContain('## `a.ts` (3 references)\n\n- `a` (fn, line 2, 3 refs)')
    expect(map.text).toContain('## `b.ts` (1 references)')
  })

  it('leaves out the least referenced files that pass its cap, and says how many', () => {
    const long = { ...ranking, files: [ranking.files[0] ?? { path: 'a.ts', refs: 3 }, { path: `${'b'.repeat(300)}.ts`, refs: 1 }] }
    const capBytes = Buffer.byteLength(renderCodeNavMap(long, { capBytes: 100_000, commit: 'c1' }).text) - 1

    const map = renderCodeNavMap(long, { capBytes, commit: 'c1' })

    expect(map.listed).toBe(1)
    expect(map.text).toContain('left out the 1 least referenced files')
    expect(Buffer.byteLength(map.text)).toBeLessThanOrEqual(capBytes)
  })

  it('says so when code-nav listed nothing', () => {
    expect(renderCodeNavMap({ files: [], symbols: [] }, { capBytes: 4000, commit: 'c1' }).text).toContain('code-nav listed no symbol')
  })
})
