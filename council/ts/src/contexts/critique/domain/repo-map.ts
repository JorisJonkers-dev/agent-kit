import { mentions, resolveImport, scanSource, type SourceScan, type SourceSymbol } from './source-scan.js'
import { bytes } from './tree-sample.js'

/** A tracked text file, by repository path. */
export interface SourceText {
  readonly path: string
  readonly text: string
}

/** A file the scan read, and the tracked files its imports name. */
export interface ScannedFile extends SourceText {
  readonly scan: SourceScan
  readonly imports: readonly string[]
}

/**
 * Which source file imports which, and where their top-level symbols are,
 * from the symbol and import scan. A file references a symbol of another
 * when it imports that file and names the symbol as a whole word.
 */
export class SourceGraph {
  private readonly importersOf = new Map<string, string[]>()

  private constructor(readonly files: ReadonlyMap<string, ScannedFile>) {
    for (const file of files.values()) {
      for (const target of file.imports) this.importersOf.set(target, [...(this.importersOf.get(target) ?? []), file.path])
    }
  }

  /** Scans every file in a language the scan knows; other files are not in the graph. */
  static build(texts: readonly SourceText[]): SourceGraph {
    const tracked = new Set(texts.map((file) => file.path))
    const files = new Map<string, ScannedFile>()
    for (const { path, text } of texts) {
      const scan = scanSource(path, text)
      if (scan === undefined) continue
      const resolved = scan.imports.flatMap((specifier) => resolveImport(path, specifier, tracked) ?? [])
      const imports = [...new Set(resolved)].filter((target) => target !== path).sort()
      files.set(path, { imports, path, scan, text })
    }
    return new SourceGraph(files)
  }

  /** The files that import a file, sorted. */
  importers(path: string): readonly string[] {
    return [...(this.importersOf.get(path) ?? [])].sort()
  }

  /** The other files that import a file and name one of its symbols. */
  references(path: string, name: string): readonly string[] {
    return this.importers(path).filter((importer) => mentions(this.files.get(importer)?.text ?? '', name))
  }

  /** The top-level symbol whose lines hold a line of a file. */
  enclosing(path: string, line: number): SourceSymbol | undefined {
    return this.files.get(path)?.scan.symbols.find((symbol) => symbol.first <= line && line <= symbol.last)
  }

  /** Where a name a file uses is declared: in the file itself, else in the first file it imports that declares it. */
  definition(path: string, name: string): { readonly path: string; readonly symbol: SourceSymbol } | undefined {
    const file = this.files.get(path)
    for (const candidate of [path, ...(file?.imports ?? [])]) {
      const symbol = this.files.get(candidate)?.scan.symbols.find((declared) => declared.name === name)
      if (symbol !== undefined) return { path: candidate, symbol }
    }
    return undefined
  }
}


/** One symbol of the repo map. */
export interface RepoMapSymbol {
  readonly name: string
  readonly kind: string
  readonly signature: string
  /** The line its declaration starts on. */
  readonly line: number
  /** How many other files reference it. */
  readonly references: number
}

/** One source file of the repo map, with its symbols, most referenced first. */
export interface RepoMapEntry {
  readonly path: string
  /** How many other files reference one of its symbols. */
  readonly references: number
  readonly symbols: readonly RepoMapSymbol[]
}

/**
 * Every source file the scan reads, ranked by how many other files
 * reference its symbols, most first, then by path. Built from the graph
 * alone, so the repo map file and the critics' `symbol` lookup read the same
 * ranking.
 */
export function buildRepoMap(graph: SourceGraph): readonly RepoMapEntry[] {
  const entries = [...graph.files.values()].map((file): RepoMapEntry => {
    const referencing = new Set<string>()
    const symbols = file.scan.symbols.map((symbol): RepoMapSymbol => {
      const references = graph.references(file.path, symbol.name)
      for (const reference of references) referencing.add(reference)
      return { kind: symbol.kind, line: symbol.first, name: symbol.name, references: references.length, signature: symbol.signature }
    })
    return {
      path: file.path,
      references: referencing.size,
      symbols: symbols.sort((a, b) => b.references - a.references || a.line - b.line),
    }
  })
  return entries.sort((a, b) => b.references - a.references || (a.path < b.path ? -1 : 1))
}

/** The repo map as frozen, and how many files its cap left out. */
export interface RenderedRepoMap {
  readonly text: string
  readonly listed: number
  readonly dropped: number
}

/** The packet path of the repo map. */
export const REPO_MAP_PATH = 'tree/REPO-MAP.md'

/**
 * The repo map as a packet file: a heading per file, in rank order, with a
 * line per symbol. Files go in while the map fits under its cap; the rest,
 * the least referenced, are left out and counted at its end.
 */
export function renderRepoMap(entries: readonly RepoMapEntry[], options: { readonly commit: string; readonly capBytes: number }): RenderedRepoMap {
  const head = [
    '# Repo map',
    '',
    `Every source file the runner's symbol scan reads at commit \`${options.commit}\`, with its top-level symbols and their signatures. Files are ranked by how many other files reference their symbols, most first; a file references a symbol when it imports the symbol's file and names it. Each symbol's line is its line in \`tree/<path>\`, which holds the file when the packet cap let it in. This map is a fact the runner computed.`,
    '',
  ].join('\n')
  const blocks = entries.map(renderEntry)
  let size = bytes(head)
  let listed = 0
  for (const block of blocks) {
    // The footer is sized for the most files it could count, so the map never passes its cap.
    const footer = listed + 1 === blocks.length ? '' : droppedLine(blocks.length, options.capBytes)
    if (size + bytes(block) + bytes(footer) > options.capBytes) break
    size += bytes(block)
    listed += 1
  }
  const dropped = blocks.length - listed
  const tail = dropped === 0 ? (blocks.length === 0 ? 'No source file in a language the scan reads.\n' : '') : droppedLine(dropped, options.capBytes)
  return { dropped, listed, text: `${head}${blocks.slice(0, listed).join('')}${tail}` }
}

function renderEntry(entry: RepoMapEntry): string {
  const symbols = entry.symbols.map(
    (symbol) => `- \`${symbol.name}\` (${symbol.kind}, line ${String(symbol.line)}, ${String(symbol.references)} refs): \`${symbol.signature}\``,
  )
  return `\n## \`${entry.path}\` (referenced by ${String(entry.references)} files)\n\n${symbols.length === 0 ? 'No top-level symbols.' : symbols.join('\n')}\n`
}

function droppedLine(count: number, cap: number): string {
  return `\nThe map's cap of ${String(cap)} bytes left out the ${String(count)} least referenced files.\n`
}
