import { languageOf, resolveImport, scanImports, type ScanLanguage } from './import-scan.js'

/** One file's history as the runner reads it from git: commit ids and committer dates, never an author or a message. */
export interface FileHistory {
  readonly path: string
  /** Newest first. */
  readonly commits: readonly { readonly commit: string; readonly date: string }[]
}

export interface HistoryFact {
  readonly path: string
  readonly last_commit: string | null
  /** The committer date of the last commit, as git prints it (ISO 8601); never today's. */
  readonly last_date: string | null
  /** How many commits changed the path. */
  readonly churn: number
}

export function historyFacts(histories: readonly FileHistory[]): readonly HistoryFact[] {
  return histories.map(({ path, commits }) => ({
    churn: commits.length,
    last_commit: commits[0]?.commit ?? null,
    last_date: commits[0]?.date ?? null,
    path,
  }))
}

/** A module's source at the commit; null when the commit has no such file. */
export interface ModuleSource {
  readonly path: string
  readonly text: string | null
}

export interface ImportFact {
  readonly specifier: string
  /** The tree file the specifier names; null for a package, a standard-library module, or a Go package directory. */
  readonly resolved: string | null
}

export interface ModuleFact {
  readonly path: string
  /** The scan used; null when the language is not known, and then no import facts were computed. */
  readonly language: ScanLanguage | null
  readonly imports: readonly ImportFact[]
  /** How many named modules import this one. */
  readonly fan_in: number
  /** How many named modules this one imports. */
  readonly fan_out: number
  /** fan_out / (fan_in + fan_out) over the named modules; null for a module with neither. */
  readonly instability: number | null
}

export interface CodeFacts {
  readonly modules: readonly ModuleFact[]
  /** Each import cycle among the named modules, its members sorted. */
  readonly cycles: readonly (readonly string[])[]
}

/** The notes of the facts index: what was not computed, and why. */
export interface FactsIndex {
  readonly notes: readonly string[]
}

export function codeFacts(sources: readonly ModuleSource[], tree: ReadonlySet<string>): CodeFacts & FactsIndex {
  const notes: string[] = []
  const scanned = sources.map(({ path, text }) => {
    const language = languageOf(path)
    if (language === null) notes.push(`${path}: no import facts were computed (the import scan does not know this language)`)
    else if (text === null) notes.push(`${path}: no import facts were computed (the file is not in the commit)`)
    const imports: ImportFact[] =
      language === null || text === null
        ? []
        : scanImports(language, text).map((found) => ({ resolved: resolveImport(language, path, found, tree), specifier: found.specifier }))
    return { imports, language, path }
  })
  const named = new Set(sources.map((source) => source.path))
  const edges = new Map(
    scanned.map(({ path, imports }) => [
      path,
      [...new Set(imports.flatMap(({ resolved }) => (resolved !== null && resolved !== path && named.has(resolved) ? [resolved] : [])))].sort(),
    ]),
  )
  const modules = scanned.map(({ path, language, imports }): ModuleFact => {
    const fanOut = edges.get(path)?.length ?? 0
    const fanIn = [...edges.values()].filter((targets) => targets.includes(path)).length
    return { fan_in: fanIn, fan_out: fanOut, imports, instability: fanIn + fanOut === 0 ? null : fanOut / (fanIn + fanOut), language, path }
  })
  return { cycles: cycles(edges), modules, notes }
}

/** The strongly connected components of more than one module (Tarjan), each sorted, and in order of their first member. */
function cycles(edges: ReadonlyMap<string, readonly string[]>): readonly (readonly string[])[] {
  const index = new Map<string, number>()
  const low = new Map<string, number>()
  const stack: string[] = []
  const found: string[][] = []
  const visit = (node: string): void => {
    index.set(node, index.size)
    low.set(node, index.get(node) ?? 0)
    stack.push(node)
    for (const next of edges.get(node) ?? []) {
      if (!index.has(next)) {
        visit(next)
        low.set(node, Math.min(low.get(node) ?? 0, low.get(next) ?? 0))
      } else if (stack.includes(next)) low.set(node, Math.min(low.get(node) ?? 0, index.get(next) ?? 0))
    }
    if (low.get(node) === index.get(node)) {
      const component = stack.splice(stack.indexOf(node))
      if (component.length > 1) found.push(component.sort())
    }
  }
  for (const node of [...edges.keys()].sort()) if (!index.has(node)) visit(node)
  // Plain code-unit order, so the output never depends on the machine's locale.
  return found.sort((a, b) => ((a[0] ?? '') < (b[0] ?? '') ? -1 : 1))
}
