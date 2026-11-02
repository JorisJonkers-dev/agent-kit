import { posix } from 'node:path'

/**
 * The symbol and import scan behind a code packet's repo map and context
 * slices: by rule, from the text alone, with no compiler and no model, so
 * the same commit always gives the same scan. Each language the scan knows
 * is one `LanguageScanner`; #233 adds Kotlin, Go and Rust beside these.
 */

/** A top-level declaration of a source file, with the lines it spans. */
export interface SourceSymbol {
  readonly name: string
  /** The declaration keyword: `function`, `class`, `const`, `def`, ... */
  readonly kind: string
  /** Its header, without its body, on one line. */
  readonly signature: string
  /** The line its declaration starts on, from 1. */
  readonly first: number
  /** The last line of its body. */
  readonly last: number
}

/** What the scan reads from one source file. */
export interface SourceScan {
  readonly language: string
  /** Its top-level symbols, in line order. */
  readonly symbols: readonly SourceSymbol[]
  /** The modules it imports, as written. */
  readonly imports: readonly string[]
}

/** One language the scan knows. */
export interface LanguageScanner {
  readonly language: string
  readonly extensions: readonly string[]
  /** The top-level declaration a line at column 0 starts, if it starts one. */
  readonly declaration: (line: string) => { readonly kind: string; readonly name: string } | undefined
  /** The declaration's header, from the lines it starts on, without its body. */
  readonly signature: (header: string, kind: string) => string
  readonly imports: (text: string) => readonly string[]
  /** The tracked path an import names, if it names one. */
  readonly resolve: (from: string, specifier: string, tracked: ReadonlySet<string>) => string | undefined
}

/** How many lines a declaration's header may run over. */
const HEADER_LINES = 6
/** The longest signature the repo map shows. */
const SIGNATURE_CHARS = 200

const TS_EXTENSIONS = ['.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs']
const TS_DECLARATION =
  /^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|class|interface|type|enum|const\s+enum|const|let|var|namespace)\s+([A-Za-z_$][\w$]*)/
const TS_IMPORT = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*)['"]([^'"\n]+)['"]/g

const typescript: LanguageScanner = {
  declaration: (line) => {
    const match = TS_DECLARATION.exec(line)
    return match === null ? undefined : { kind: (match[1] ?? '').replace(/\s+/g, ' '), name: match[2] ?? '' }
  },
  extensions: TS_EXTENSIONS,
  imports: (text) => [...text.matchAll(TS_IMPORT)].map((match) => match[1] ?? ''),
  language: 'typescript',
  resolve: (from, specifier, tracked) => {
    if (!specifier.startsWith('.')) return undefined
    const target = posix.join(posix.dirname(from), specifier)
    const stem = target.replace(/\.(?:[mc]?js|jsx)$/, '')
    const candidates = [target, ...TS_EXTENSIONS.map((ext) => `${stem}${ext}`), ...TS_EXTENSIONS.map((ext) => `${target}/index${ext}`)]
    return candidates.find((candidate) => tracked.has(candidate))
  },
  signature: (header, kind) => {
    if (kind === 'type') return firstLine(header)
    if (kind === 'const' || kind === 'let' || kind === 'var') return valueSignature(header)
    return cutAt(header, '{')
  },
}

const PY_DECLARATION = /^(?:(async\s+def|def|class)\s+([A-Za-z_]\w*)|([A-Za-z_]\w*)\s*(?::[^=]*)?=(?!=))/
const PY_FROM = /^\s*from\s+(\.*[\w.]*)\s+import\s+(?:\(([^)]*)\)|([^#\n]*))/gm
const PY_IMPORT = /^\s*import\s+([\w.]+(?:\s*,\s*[\w.]+)*)/gm

const python: LanguageScanner = {
  declaration: (line) => {
    const match = PY_DECLARATION.exec(line)
    if (match === null) return undefined
    return match[3] === undefined ? { kind: (match[1] ?? '').replace(/\s+/g, ' '), name: match[2] ?? '' } : { kind: 'assignment', name: match[3] }
  },
  extensions: ['.py'],
  imports: (text) => [
    ...[...text.matchAll(PY_FROM)].flatMap((match) => {
      const module = match[1] ?? ''
      const names = (match[2] ?? match[3] ?? '').split(',').map((name) => name.trim().split(/\s+/)[0] ?? '').filter((name) => /^\w+$/.test(name))
      return [module, ...names.map((name) => `${module}${module.endsWith('.') ? '' : '.'}${name}`)]
    }),
    ...[...text.matchAll(PY_IMPORT)].flatMap((match) => (match[1] ?? '').split(',').map((name) => name.trim())),
  ],
  language: 'python',
  resolve: (from, specifier, tracked) => {
    const dots = /^\.*/.exec(specifier)?.[0].length ?? 0
    const module = specifier.slice(dots).split('.').filter((part) => part !== '').join('/')
    const files = (base: string) => (base === '' ? [] : [`${base}.py`, `${base}/__init__.py`])
    if (dots > 0) {
      const dir = Array.from({ length: dots - 1 }).reduce<string>((path) => posix.dirname(path), posix.dirname(from))
      const base = module === '' ? dir : posix.join(dir, module)
      return files(base === '.' ? '' : base).find((candidate) => tracked.has(candidate))
    }
    const exact = files(module).find((candidate) => tracked.has(candidate))
    if (exact !== undefined) return exact
    const nested = [...tracked].filter((path) => files(module).some((candidate) => path.endsWith(`/${candidate}`)))
    return nested.length === 1 ? nested[0] : undefined
  },
  signature: (header, kind) => (kind === 'assignment' ? cutAt(header, '=') : cutAt(header, ':')),
}

/** Every language the scan knows; the import scan for Kotlin, Go and Rust (#233) adds its scanners here. */
export const SCANNERS: readonly LanguageScanner[] = [typescript, python]

/** The scanner for a path, by its extension. */
export function scannerFor(path: string): LanguageScanner | undefined {
  const extension = posix.extname(path)
  return SCANNERS.find((scanner) => scanner.extensions.includes(extension))
}

/** The tracked file an import of a file names, by the file's language. */
export function resolveImport(from: string, specifier: string, tracked: ReadonlySet<string>): string | undefined {
  return scannerFor(from)?.resolve(from, specifier, tracked)
}

/**
 * The top-level symbols and imports of a file in a language the scan knows.
 * A top-level declaration starts at column 0 and runs to the last line
 * before the next line at column 0 that does not close a bracket, without
 * the blank lines at its end.
 */
export function scanSource(path: string, text: string): SourceScan | undefined {
  const scanner = scannerFor(path)
  if (scanner === undefined) return undefined
  const lines = text.split('\n')
  const starts = lines.flatMap((line, index) => (startsStatement(line) ? [index] : []))
  const symbols = starts.flatMap((start, i): SourceSymbol[] => {
    const line = lines[start] ?? ''
    const declared = scanner.declaration(line)
    if (declared === undefined) return []
    let end = (starts[i + 1] ?? lines.length) - 1
    while (end > start && (lines[end] ?? '').trim() === '') end -= 1
    const header = lines.slice(start, Math.min(end, start + HEADER_LINES - 1) + 1).join('\n')
    return [{ first: start + 1, kind: declared.kind, last: end + 1, name: declared.name, signature: shorten(scanner.signature(header, declared.kind)) }]
  })
  return { imports: scanner.imports(text), language: scanner.language, symbols }
}

/** Whether a line starts a top-level statement: it starts at column 0 and does not close a bracket. */
function startsStatement(line: string): boolean {
  return /^[^\s)\]}]/.test(line)
}

/** Whether a text mentions a name as a whole word. */
export function mentions(text: string, name: string): boolean {
  return wordPattern(name).test(text)
}

/** The lines, from 1, of a text that mention a name as a whole word. */
export function mentionLines(text: string, name: string): readonly number[] {
  const pattern = wordPattern(name)
  return text.split('\n').flatMap((line, index) => (pattern.test(line) ? [index + 1] : []))
}

function wordPattern(name: string): RegExp {
  return new RegExp(`(?<![\\w$])${name.replace(/\$/g, '\\$')}(?![\\w$])`)
}

function firstLine(header: string): string {
  return header.split('\n')[0] ?? ''
}

/** The header up to the first `stop` outside brackets. */
function cutAt(header: string, stop: string): string {
  let depth = 0
  for (let i = 0; i < header.length; i += 1) {
    const char = header.charAt(i)
    if ('([<'.includes(char)) depth += 1
    else if (')]>'.includes(char) && header.charAt(i - 1) !== '=') depth = Math.max(0, depth - 1)
    else if (char === stop && depth === 0 && !(stop === '=' && '=>'.includes(header.charAt(i + 1)))) return header.slice(0, i)
  }
  return header
}

/** A variable's name and type, and an arrow function's parameters and return type. */
function valueSignature(header: string): string {
  const name = cutAt(header, '=')
  const value = header.slice(name.length + 1).trimStart()
  if (!/^(?:async\s+)?(?:\(|<|[A-Za-z_$][\w$]*\s*=>)/.test(value)) return name
  const arrow = value.indexOf('=>')
  return arrow === -1 ? name : `${name.trimEnd()} = ${value.slice(0, arrow + 2)}`
}

function shorten(signature: string): string {
  const flat = signature.replace(/\s+/g, ' ').trim()
  return flat.length <= SIGNATURE_CHARS ? flat : `${flat.slice(0, SIGNATURE_CHARS - 1)}…`
}
