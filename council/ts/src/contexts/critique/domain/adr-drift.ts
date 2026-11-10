import { decisionDate, withoutFences } from './decision-docs.js'

/** A record names code that is gone: a path missing from the working tree, or a symbol no file of it holds. */
export interface AdrDrift {
  readonly record: string
  /** The path or the symbol, as the record writes it. */
  readonly named: string
  readonly reason: 'missing-path' | 'missing-symbol'
}

/** A record's decision is older than the last change to a file it names. */
export interface AdrStale {
  readonly record: string
  readonly path: string
  /** The day the record states its decision was made, `YYYY-MM-DD`. */
  readonly decision_date: string
  /** The date of the last commit that changed the path, ISO 8601. */
  readonly last_change: string
  readonly last_commit: string
}

/** The code, configuration and commands a record names in backticks. */
export interface NamedCode {
  /** Repository paths, without a leading `./` or a trailing `/`. */
  readonly paths: readonly string[]
  readonly symbols: readonly string[]
}

const KNOWN_EXTENSION = /\.(?:ts|tsx|js|mjs|cjs|json|ya?ml|toml|md|py|sh|css|html|sql|lock|kt|java|go|rs)$/i
const PATH_SHAPE = /^[\w@.-]+(?:\/[\w@.-]+)*\/?$/
const IDENTIFIER = /^[A-Za-z_]\w*$/

/** The paths and symbols a record names inside backticks, outside fenced blocks, in order and without repeats. */
export function namedCode(markdown: string): NamedCode {
  const paths = new Set<string>()
  const symbols = new Set<string>()
  for (const match of withoutFences(markdown).matchAll(/`([^`\n]+)`/g)) {
    const [pathPart = '', symbolPart] = (match[1] ?? '').trim().split('#')
    const path = pathPart.replace(/^\.\//, '').replace(/\/+$/, '')
    if (isPath(pathPart)) {
      paths.add(path)
      const anchor = symbolPart?.replace(/\(\)$/, '')
      if (anchor !== undefined && IDENTIFIER.test(anchor)) symbols.add(anchor)
    } else if (symbolPart === undefined && isSymbol(pathPart)) {
      symbols.add(pathPart.replace(/\(\)$/, ''))
    }
  }
  return { paths: [...paths], symbols: [...symbols] }
}

/**
 * A path has a known extension, a trailing `/` or a leading `./`: a bare
 * `yes/no` or `read/write` is prose, not a path the tree must hold.
 */
function isPath(text: string): boolean {
  const path = text.replace(/^\.\//, '')
  const marked = path !== text || path.endsWith('/') || KNOWN_EXTENSION.test(path)
  return marked && PATH_SHAPE.test(path) && !path.split('/').includes('..')
}

/**
 * An identifier with an internal hump (camelCase or PascalCase), with or
 * without `()`. A bare `fetch()` or a snake_case key such as `max_tokens` is
 * too common in prose and configuration to count; name it as `path#symbol`.
 */
function isSymbol(text: string): boolean {
  const name = text.replace(/\(\)$/, '')
  return IDENTIFIER.test(name) && /[a-z0-9][A-Z]/.test(name)
}

/** What the working tree holds, as the drift and stale checks ask for it. */
export interface CodeFacts {
  /** Whether a repository path exists in the working tree. */
  readonly pathExists: (path: string) => boolean
  /** Whether a symbol appears as a whole word in a file of the tree outside the decision log. */
  readonly symbolExists: (symbol: string) => boolean
  /** The last commit that changed an existing path; absent for a path no commit touched. */
  readonly lastChange: (path: string) => { readonly commit: string; readonly date: string } | undefined
}

/** One record to check: its path, its text, and whether it is in force. */
export interface CodeCheckedRecord {
  readonly path: string
  readonly text: string
}

/** The code each record names, which the caller resolves against the tree before `driftAndStale` judges it. */
export function namedBy(records: readonly CodeCheckedRecord[]): readonly (CodeCheckedRecord & NamedCode)[] {
  return records.map((record) => ({ ...record, ...namedCode(record.text) }))
}

/**
 * Drift: a record in force names a path or a symbol the tree no longer
 * holds. Stale: it states a decision date, and a path it names changed on a
 * later day. A record that names no code is never flagged.
 */
export function driftAndStale(
  records: readonly (CodeCheckedRecord & NamedCode)[],
  facts: CodeFacts,
): { readonly drifted: readonly AdrDrift[]; readonly stale: readonly AdrStale[] } {
  const drifted: AdrDrift[] = []
  const stale: AdrStale[] = []
  for (const record of records) {
    const missing = record.paths.filter((path) => !facts.pathExists(path))
    drifted.push(
      ...missing.map((named): AdrDrift => ({ named, reason: 'missing-path', record: record.path })),
      ...record.symbols
        .filter((named) => !facts.symbolExists(named))
        .map((named): AdrDrift => ({ named, reason: 'missing-symbol', record: record.path })),
    )
    const decided = decisionDate(record.text)
    if (decided === undefined) continue
    const logDir = record.path.includes('/') ? record.path.slice(0, record.path.lastIndexOf('/') + 1) : ''
    // A record naming another decision (or itself) is not naming code: editing a record never makes it stale.
    for (const path of record.paths.filter((named) => !missing.includes(named) && !(logDir !== '' && named.startsWith(logDir)))) {
      const change = facts.lastChange(path)
      if (change !== undefined && change.date.slice(0, 10) > decided)
        stale.push({ decision_date: decided, last_change: change.date, last_commit: change.commit, path, record: record.path })
    }
  }
  return { drifted, stale }
}
