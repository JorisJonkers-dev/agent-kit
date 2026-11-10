import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { harness, type Harness } from '../../../test/critique/harness.js'
import { codeFacts } from './domain/code-facts.js'
import { languageOf, resolveImport, scanImports, type ScannedImport } from './domain/import-scan.js'
import type { FactsResult, Refusal } from './index.js'

/**
 * The runner's history and import-graph facts (#479, ADR-0004): computed from
 * the commit, never by a critic, and the same for the same commit.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const DAY_1 = '2026-01-01T10:00:00+00:00'
const DAY_2 = '2026-02-02T11:00:00+00:00'
const DAY_3 = '2026-03-03T12:00:00+00:00'

/** Three commits: a.ts and b.ts first, a.ts again, then b.ts and the other languages. */
async function fixture(): Promise<{ h: Harness; commits: readonly string[] }> {
  const h = await harness()
  open.push(h)
  const write = (name: string, text: string): Promise<void> => h.writeText(join(h.repo, name), text)
  await write('src/a.ts', "import { b } from './b.js'\nimport fs from 'node:fs'\nexport const a = b\n")
  await write('src/b.ts', "export const b = 1\n")
  await write('docs/notes.txt', 'plain notes\n')
  const c1 = await h.git.commit(DAY_1)
  await write('src/a.ts', "import { b } from './b.js'\nimport fs from 'node:fs'\nexport const a = b + 1\n")
  const c2 = await h.git.commit(DAY_2)
  await write('src/b.ts', "import { a } from './a.js'\nexport const b = 2\n")
  await write('py/app.py', 'import os\nfrom .util import helper\n')
  await write('py/util.py', 'helper = 1\n')
  await write('kt/Main.kt', 'package kt\nimport kt.util.Tools\n')
  await write('kt/util/Tools.kt', 'package kt.util\n')
  await write('go/main.go', 'package main\n\nimport (\n\t"fmt"\n\tu "example.com/x/util"\n)\n')
  await write('rs/lib.rs', 'use std::fmt;\nmod helper;\n')
  await write('rs/helper.rs', 'pub fn f() {}\n')
  const c3 = await h.git.commit(DAY_3)
  return { commits: [c1, c2, c3], h }
}

/** Every key of every object in a JSON value. */
function keysOf(value: unknown): readonly string[] {
  if (Array.isArray(value)) return value.flatMap(keysOf)
  if (typeof value !== 'object' || value === null) return []
  return Object.entries(value).flatMap(([key, inner]) => [key, ...keysOf(inner)])
}

async function facts(h: Harness, input: object = {}): Promise<FactsResult> {
  const result = await h.commands.facts.run({ repo: h.repo, ...input })
  if (result.status === 'refused') throw new Error(`facts refused: ${result.message}`)
  return result
}

describe('history facts', () => {
  it('report the last commit that changed a named file, its own date and how many commits changed it', async () => {
    const { h, commits } = await fixture()

    const result = await facts(h, { files: ['src/b.ts', 'src/a.ts', 'missing.ts'] })

    expect(result.commit).toBe(commits[2])
    expect(result.history).toEqual([
      { churn: 0, last_commit: null, last_date: null, path: 'missing.ts' },
      { churn: 2, last_commit: commits[1], last_date: DAY_2, path: 'src/a.ts' },
      { churn: 2, last_commit: commits[2], last_date: DAY_3, path: 'src/b.ts' },
    ])
  })
})

describe('code facts', () => {
  it('report a TypeScript module imports, with the tree file each resolves to, though fan-out counts the named set only', async () => {
    const { h } = await fixture()

    const result = await facts(h, { modules: ['src/a.ts'] })

    expect(result.modules).toEqual([
      {
        fan_in: 0,
        fan_out: 0,
        imports: [
          { resolved: 'src/b.ts', specifier: './b.js' },
          { resolved: null, specifier: 'node:fs' },
        ],
        instability: null,
        language: 'typescript',
        path: 'src/a.ts',
      },
    ])
    expect(result.cycles).toEqual([])
  })

  it('count fan-in, fan-out and instability over the named set, and report its import cycles', async () => {
    const { h } = await fixture()

    const result = await facts(h, { modules: ['src/a.ts', 'src/b.ts'] })

    expect(result.modules.map(({ path, fan_in, fan_out, instability }) => ({ fan_in, fan_out, instability, path }))).toEqual([
      { fan_in: 1, fan_out: 1, instability: 0.5, path: 'src/a.ts' },
      { fan_in: 1, fan_out: 1, instability: 0.5, path: 'src/b.ts' },
    ])
    expect(result.cycles).toEqual([['src/a.ts', 'src/b.ts']])
  })

  it('scan Python, Kotlin, Go and Rust modules', async () => {
    const { h } = await fixture()

    const result = await facts(h, { modules: ['py/app.py', 'kt/Main.kt', 'go/main.go', 'rs/lib.rs'] })

    const imports = Object.fromEntries(result.modules.map((module) => [module.path, [module.language, module.imports]]))
    expect(imports).toEqual({
      'go/main.go': [
        'go',
        [
          { resolved: null, specifier: 'fmt' },
          { resolved: null, specifier: 'example.com/x/util' },
        ],
      ],
      'kt/Main.kt': ['kotlin', [{ resolved: 'kt/util/Tools.kt', specifier: 'kt.util.Tools' }]],
      'py/app.py': [
        'python',
        [
          { resolved: null, specifier: 'os' },
          { resolved: 'py/util.py', specifier: '.util' },
        ],
      ],
      'rs/lib.rs': [
        'rust',
        [
          { resolved: null, specifier: 'std::fmt' },
          { resolved: 'rs/helper.rs', specifier: 'helper' },
        ],
      ],
    })
    expect(result.notes).toEqual([])
  })

  it('give a file in an unknown language no import facts, and say so in the index', async () => {
    const { h } = await fixture()

    const result = await facts(h, { modules: ['docs/notes.txt', 'nowhere.ts'] })

    expect(result.modules).toEqual([
      { fan_in: 0, fan_out: 0, imports: [], instability: null, language: null, path: 'docs/notes.txt' },
      { fan_in: 0, fan_out: 0, imports: [], instability: null, language: 'typescript', path: 'nowhere.ts' },
    ])
    expect(result.notes).toEqual([
      'docs/notes.txt: no import facts were computed (the import scan does not know this language)',
      'nowhere.ts: no import facts were computed (the file is not in the commit)',
    ])
  })
})

describe('the facts command', () => {
  it('prints the same bytes twice, and holds no author or commit message', async () => {
    const { h } = await fixture()
    const input = { files: ['src/a.ts', 'src/b.ts'], modules: ['src/a.ts', 'src/b.ts'] }

    const first = JSON.stringify(await facts(h, input))
    const second = JSON.stringify(await facts(h, { files: [...input.files].reverse(), modules: [...input.modules].reverse() }))

    expect(second).toBe(first)
    expect(Object.keys(JSON.parse(first) as object).sort()).toEqual(['commit', 'cycles', 'history', 'modules', 'notes', 'status'])
    expect(keysOf(JSON.parse(first) as unknown)).not.toContain('author')
    expect(keysOf(JSON.parse(first) as unknown)).not.toContain('message')
  })

  it('gives identical bytes for two fresh repositories at the same commit', async () => {
    const input = { files: ['src/a.ts', 'src/b.ts'], modules: ['src/a.ts', 'src/b.ts', 'py/app.py'] }

    const one = JSON.stringify(await facts((await fixture()).h, input))
    const two = JSON.stringify(await facts((await fixture()).h, input))

    expect(two).toBe(one)
  })

  it('reads history at the pinned commit, not at what HEAD has since become', async () => {
    const { h, commits } = await fixture()

    expect(await h.git.fileCommits(h.repo, commits[1] ?? '', 'src/b.ts')).toEqual([{ commit: commits[0], date: DAY_1 }])
  })

  it('is refused for a repository with no commit', async () => {
    const h = await harness()
    open.push(h)
    await h.git.orphan('empty')

    const result: FactsResult | Refusal = await h.commands.facts.run({ repo: h.repo })

    expect(result).toMatchObject({ reason: 'no-commit', status: 'refused' })
  })

  it('is refused where the allowlist refuses the repository', async () => {
    const h = await harness({ remotes: [] })
    open.push(h)

    expect(await h.commands.facts.run({ repo: h.repo })).toMatchObject({ status: 'refused' })
  })
})

const specifiers = (found: readonly { readonly specifier: string }[]): readonly string[] => found.map((entry) => entry.specifier)
const use = (specifier: string): ScannedImport => ({ kind: 'import', specifier })
const mod = (specifier: string): ScannedImport => ({ kind: 'module', specifier })

describe('the import scan', () => {
  it('knows a language by its extension only', () => {
    expect(languageOf('a/b.mjs')).toBe('typescript')
    expect(languageOf('a.b/Makefile')).toBeNull()
    expect(languageOf('README')).toBeNull()
  })

  it('reads every TypeScript import form and skips comments', () => {
    const text = [
      "import a from './a'",
      "import './side'",
      "export { x } from '../x'",
      "import type { T } from 'pkg'",
      "// import skipped from './skipped'",
      "/* import also from './also'",
      "   import more from './more' */",
      "const lazy = await import('./lazy')",
      "const c = require('./c')",
      "import a2 from './a'",
    ].join('\n')

    expect(specifiers(scanImports('typescript', text))).toEqual(['./a', './side', '../x', 'pkg', './lazy', './c'])
  })

  it('reads Python import forms, Kotlin imports, Go single and block imports, and Rust use and mod', () => {
    expect(specifiers(scanImports('python', '# import no\nimport os, sys\nfrom a.b import c\nfrom .. import d\nimport e.f'))).toEqual([
      'os',
      'sys',
      'a.b',
      '..',
      'e.f',
    ])
    expect(specifiers(scanImports('kotlin', '// import no\nimport a.b.C\nimport a.b.*'))).toEqual(['a.b.C', 'a.b'])
    expect(specifiers(scanImports('go', 'import "one"\nimport (\n\t"two"\n\t_ "three"\n)\nvar s = "not"\n\t"nor"\nimport x "four"'))).toEqual([
      'one',
      'two',
      'three',
      'four',
    ])
    expect(scanImports('rust', '// use no::no;\nuse a::b;\npub use c::d::{e, f};\nmod g;\npub mod h;')).toEqual([
      { kind: 'import', specifier: 'a::b' },
      { kind: 'import', specifier: 'c::d' },
      { kind: 'module', specifier: 'g' },
      { kind: 'module', specifier: 'h' },
    ])
  })

  it('resolves relative specifiers inside the tree only', () => {
    const tree = new Set(['src/a.ts', 'src/dir/index.ts', 'src/c.tsx', 'x/mod.rs', 'p/q/__init__.py', 'p/r.py', 'top.py', 'k/A.kt', 'A.kt', 'm/B.kt', 'n/B.kt', 'src/foo/bar/x.rs', 'src/foo/bar/z/mod.rs', 'src/foo/w.rs'])

    expect(resolveImport('typescript', 'src/z.ts', use('./a.js'), tree)).toBe('src/a.ts')
    expect(resolveImport('typescript', 'src/z.ts', use('./dir'), tree)).toBe('src/dir/index.ts')
    expect(resolveImport('typescript', 'src/z.ts', use('./c'), tree)).toBe('src/c.tsx')
    expect(resolveImport('typescript', 'src/z.ts', use('../../up'), tree)).toBeNull()
    expect(resolveImport('typescript', 'src/z.ts', use('pkg'), tree)).toBeNull()
    expect(resolveImport('python', 'p/m.py', use('.r'), tree)).toBe('p/r.py')
    expect(resolveImport('python', 'p/s/m.py', use('..r'), tree)).toBe('p/r.py')
    expect(resolveImport('python', 'p/m.py', use('.'), tree)).toBeNull()
    expect(resolveImport('python', 'm.py', use('..up'), tree)).toBeNull()
    expect(resolveImport('python', 'm.py', use('p.q'), tree)).toBe('p/q/__init__.py')
    expect(resolveImport('python', 'm.py', use('top'), tree)).toBe('top.py')
    expect(resolveImport('kotlin', 'm.kt', use('k.A'), tree)).toBe('k/A.kt')
    expect(resolveImport('kotlin', 'm.kt', use('A'), tree)).toBeNull()
    expect(resolveImport('kotlin', 'm.kt', use('gone.A'), tree)).toBeNull()
    expect(resolveImport('kotlin', 'm.kt', use('B'), tree)).toBeNull()
    expect(resolveImport('go', 'm.go', use('x/y'), tree)).toBeNull()
    expect(resolveImport('rust', 'lib.rs', mod('x'), tree)).toBe('x/mod.rs')
    expect(resolveImport('rust', 'y/lib.rs', mod('x'), tree)).toBeNull()
    expect(resolveImport('rust', 'src/foo/bar.rs', mod('x'), tree)).toBe('src/foo/bar/x.rs')
    expect(resolveImport('rust', 'src/foo/bar.rs', mod('z'), tree)).toBe('src/foo/bar/z/mod.rs')
    expect(resolveImport('rust', 'src/foo/mod.rs', mod('w'), tree)).toBe('src/foo/w.rs')
    expect(resolveImport('rust', 'lib.rs', use('x'), tree)).toBeNull()
    expect(resolveImport('rust', 'lib.rs', use('a::b'), tree)).toBeNull()
  })

  it('finds several cycles in order, and does not count an edge into a finished cycle as part of another', () => {
    const imports = (...names: string[]): string => names.map((name) => `import './${name}'`).join('\n')
    const sources = [
      { path: 'a.ts', text: imports('b') },
      { path: 'b.ts', text: imports('a') },
      { path: 'c.ts', text: imports('d') },
      { path: 'd.ts', text: imports('c') },
      { path: 'e.ts', text: imports('c', 'a') },
    ]

    expect(codeFacts(sources, new Set(sources.map((source) => source.path))).cycles).toEqual([
      ['a.ts', 'b.ts'],
      ['c.ts', 'd.ts'],
    ])
  })

  it('ignores a module that imports itself when counting edges', () => {
    const tree = new Set(['a.ts'])

    expect(codeFacts([{ path: 'a.ts', text: "import './a'" }], tree).modules[0]).toMatchObject({ fan_in: 0, fan_out: 0, instability: null })
  })
})
