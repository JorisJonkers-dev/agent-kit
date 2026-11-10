/** The languages the built-in import scan knows. */
export type ScanLanguage = 'typescript' | 'python' | 'kotlin' | 'go' | 'rust'

const EXTENSIONS: Readonly<Record<string, ScanLanguage>> = {
  '.ts': 'typescript',
  '.tsx': 'typescript',
  '.mts': 'typescript',
  '.cts': 'typescript',
  '.js': 'typescript',
  '.jsx': 'typescript',
  '.mjs': 'typescript',
  '.cjs': 'typescript',
  '.py': 'python',
  '.kt': 'kotlin',
  '.kts': 'kotlin',
  '.go': 'go',
  '.rs': 'rust',
}

/** The language a path is scanned as (TypeScript and JavaScript share one scan), or null for one the scan does not know. */
export function languageOf(path: string): ScanLanguage | null {
  const dot = path.lastIndexOf('.')
  const slash = path.lastIndexOf('/')
  return dot > slash ? (EXTENSIONS[path.slice(dot)] ?? null) : null
}

/** Strips block comments and whole-line `//` and `#` comments, keeping line breaks so a scan stays line oriented. */
function withoutComments(text: string, language: ScanLanguage): string {
  const blockless = language === 'python' ? text : text.replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, ''))
  const marker = language === 'python' ? '#' : '//'
  return blockless
    .split('\n')
    .map((line) => (line.trimStart().startsWith(marker) ? '' : line))
    .join('\n')
}

const PATTERNS: Readonly<Record<ScanLanguage, readonly RegExp[]>> = {
  typescript: [
    /^\s*(?:import|export)\b[\w\s,{}*$]*?\bfrom\s*['"]([^'"\n]+)['"]/gm,
    /^\s*import\s*['"]([^'"\n]+)['"]/gm,
    /\b(?:require|import)\s*\(\s*['"]([^'"\n]+)['"]\s*\)/g,
  ],
  python: [/^\s*from\s+(\.*[\w.]*)\s+import\b/gm, /^\s*import\s+([\w.]+(?:\s*,\s*[\w.]+)*)/gm],
  kotlin: [/^\s*import\s+([\w.]+)/gm],
  go: [/^\s*import\s+(?:[\w.]+\s+)?"([^"\n]+)"/gm, /^\s+(?:[\w.]+\s+)?"([^"\n]+)"\s*$/gm],
  rust: [/^\s*(?:pub\s+)?use\s+([\w:]+)/gm, /^\s*(?:pub\s+)?mod\s+(\w+)\s*;/gm],
}

/** One import found in a file; a `module` is a Rust `mod x;` declaration, which names a file beside or below the declaring one. */
export interface ScannedImport {
  readonly specifier: string
  readonly kind: 'import' | 'module'
}

/**
 * The imports of one file's text, in order of first appearance and
 * without repeats. Line oriented and regex based: comments are dropped, and a
 * specifier inside a string or template literal on a line of its own is not
 * told from a real one. A Go specifier is read inside an `import (...)` block
 * only.
 */
export function scanImports(language: ScanLanguage, text: string): readonly ScannedImport[] {
  const code = withoutComments(text, language)
  const found: { readonly index: number; readonly specifier: string; readonly kind: ScannedImport['kind'] }[] = []
  const lines = language === 'go' ? goImportLines(code) : code
  for (const [position, pattern] of PATTERNS[language].entries()) {
    const kind = language === 'rust' && position === 1 ? 'module' : 'import'
    for (const match of lines.matchAll(pattern)) {
      const raw = language === 'kotlin' || language === 'rust' ? (match[1] ?? '').replace(/[.:]+$/, '') : (match[1] ?? '')
      const names = language === 'python' && !raw.startsWith('.') ? raw.split(',').map((name) => name.trim()) : [raw]
      for (const specifier of names) found.push({ index: match.index, kind, specifier })
    }
  }
  const ordered = found.filter((entry) => entry.specifier !== '').sort((a, b) => a.index - b.index)
  const seen = new Set<string>()
  return ordered.flatMap(({ specifier, kind }) => {
    if (seen.has(`${kind} ${specifier}`)) return []
    seen.add(`${kind} ${specifier}`)
    return [{ kind, specifier }]
  })
}

/** Only the lines of the file's `import` declarations, so a quoted string elsewhere is not taken for an import. */
function goImportLines(code: string): string {
  const lines = code.split('\n')
  let inBlock = false
  return lines
    .map((line) => {
      if (inBlock) {
        inBlock = !/^\s*\)/.test(line)
        return inBlock ? line : ''
      }
      if (/^\s*import\s*\(/.test(line)) {
        inBlock = true
        return ''
      }
      return /^\s*import\s/.test(line) ? line : ''
    })
    .join('\n')
}

const dirOf = (path: string): string => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '')

/** Joins a directory and a relative path, resolving `.` and `..`; null when it climbs out of the tree. */
function joined(directory: string, relative: string): string | null {
  const parts = directory === '' ? [] : directory.split('/')
  for (const part of relative.split('/')) {
    if (part === '..') {
      if (parts.pop() === undefined) return null
    } else if (part !== '.' && part !== '') parts.push(part)
  }
  return parts.join('/')
}

function firstIn(tree: ReadonlySet<string>, candidates: readonly (string | null)[]): string | null {
  return candidates.find((candidate): candidate is string => candidate !== null && tree.has(candidate)) ?? null
}

/**
 * The repo-relative path of a tree file an import specifier names, or null
 * when it names none: a package, the standard library, or (for Go) a package
 * directory, which is many files rather than one.
 */
export function resolveImport(language: ScanLanguage, from: string, found: ScannedImport, tree: ReadonlySet<string>): string | null {
  const { specifier } = found
  switch (language) {
    case 'typescript': {
      if (!specifier.startsWith('.')) return null
      const base = joined(dirOf(from), specifier)
      if (base === null) return null
      const stem = base.replace(/\.(?:[cm]?js|jsx)$/, '')
      const extensions = ['.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs']
      return firstIn(tree, [
        base,
        ...extensions.map((extension) => `${stem}${extension}`),
        ...extensions.map((extension) => `${base}/index${extension}`),
      ])
    }
    case 'python': {
      const dots = /^\.*/.exec(specifier)?.[0].length ?? 0
      const directory = dots === 0 ? '' : joined(dirOf(from), Array<string>(dots - 1).fill('..').join('/'))
      if (directory === null) return null
      const base = [directory, specifier.slice(dots).split('.').join('/')].filter((part) => part !== '').join('/')
      return base === '' ? null : firstIn(tree, [`${base}.py`, `${base}/__init__.py`])
    }
    case 'kotlin': {
      const path = specifier.split('.').join('/')
      const matches = [...tree].filter((file) => file === `${path}.kt` || file.endsWith(`/${path}.kt`))
      return matches.length === 1 ? (matches[0] ?? null) : null
    }
    case 'go':
      return null
    case 'rust': {
      if (found.kind !== 'module') return null
      // `mod x;` in lib.rs, main.rs or mod.rs names a file beside it; in any other file, one below its stem's directory.
      const name = from.slice(from.lastIndexOf('/') + 1)
      const directory = ['lib.rs', 'main.rs', 'mod.rs'].includes(name) ? dirOf(from) : from.replace(/\.rs$/, '')
      return firstIn(tree, [joined(directory, `${specifier}.rs`), joined(directory, `${specifier}/mod.rs`)])
    }
  }
}
