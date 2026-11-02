import { lineCount } from './line-references.js'
import type { SourceGraph } from './repo-map.js'
import { mentionLines, type SourceSymbol } from './source-scan.js'
import { mergeRanges, type LineRange } from './tree-sample.js'

/** Why a slice is in a code packet. */
export const SLICE_REASONS = ['changed', 'callee', 'caller'] as const
export type SliceReason = (typeof SLICE_REASONS)[number]

/** Some lines of a file of the tree that a reviewer needs beside the diff. */
export interface ContextSlice {
  /** The repository path. */
  readonly path: string
  readonly first: number
  readonly last: number
  readonly reason: SliceReason
  /** The symbol the slice holds, when it holds one. */
  readonly symbol?: string
  /** For a callee or a caller: the changed symbol it is called by or calls. */
  readonly of?: string
}

/** A file the change touched that is still in the tree, with its text before the change (none for an added file). */
export interface ChangedFile {
  readonly path: string
  readonly text: string
  readonly before: string | undefined
}

/** How many lines around a change a file the scan cannot read sends. */
const CONTEXT_LINES = 10
/** How many lines around a change outside every symbol a source file sends. */
const STATEMENT_CONTEXT_LINES = 3
/** How many callers and how many callees of each changed symbol are sent. */
const RELATED_PER_SYMBOL = 8
/** Past this many cells, the line diff takes the whole changed middle of a file as changed. */
const DIFF_CELLS = 4_000_000

/**
 * The slices a code packet sends around a change, by rule and never by a
 * model: each changed hunk's enclosing top-level symbol (or, in a file the
 * scan cannot read, the hunk with ten lines either side), then each changed
 * symbol's callees (the symbols its body names, declared in its file or in
 * a file it imports), then its callers (the symbols of its file and of the
 * files that import it that name it). Each slice once, in that order.
 */
export function contextSlices(graph: SourceGraph, changed: readonly ChangedFile[]): readonly ContextSlice[] {
  const own: ContextSlice[] = []
  const symbols: { readonly path: string; readonly symbol: SourceSymbol }[] = []
  for (const file of changed) {
    const lines = changedLines(file.before, file.text)
    const scanned = graph.files.has(file.path)
    const loose: LineRange[] = []
    const enclosing = new Map<number, SourceSymbol>()
    for (const line of lines) {
      const symbol = graph.enclosing(file.path, line)
      if (symbol !== undefined) enclosing.set(symbol.first, symbol)
      else loose.push(around(line, scanned ? STATEMENT_CONTEXT_LINES : CONTEXT_LINES, lineCount(file.text)))
    }
    const found = [...enclosing.values()]
    symbols.push(...found.map((symbol) => ({ path: file.path, symbol })))
    own.push(
      ...[...found.map((symbol) => ({ first: symbol.first, last: symbol.last, symbol: symbol.name })), ...mergeRanges(loose)]
        .sort((a, b) => a.first - b.first)
        .map((range): ContextSlice => ({ ...range, path: file.path, reason: 'changed' })),
    )
  }
  const callees = symbols.flatMap(({ path, symbol }) => calleesOf(graph, path, symbol))
  const callers = symbols.flatMap(({ path, symbol }) => callersOf(graph, path, symbol))
  const seen = new Set<string>()
  return [...own, ...callees, ...callers].filter((slice) => {
    const key = `${slice.path}:${String(slice.first)}-${String(slice.last)}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

/** The symbols a changed symbol's body names, declared in its own file or in a file it imports. */
function calleesOf(graph: SourceGraph, path: string, symbol: SourceSymbol): readonly ContextSlice[] {
  const text = graph.files.get(path)?.text ?? ''
  const body = text.split('\n').slice(symbol.first - 1, symbol.last).join('\n')
  const names = [...new Set(body.match(/[A-Za-z_$][\w$]*/g) ?? [])]
  return names
    .flatMap((name): ContextSlice[] => {
      const found = graph.definition(path, name)
      if (found === undefined || (found.path === path && found.symbol.first === symbol.first)) return []
      return [{ first: found.symbol.first, last: found.symbol.last, of: symbol.name, path: found.path, reason: 'callee', symbol: name }]
    })
    .slice(0, RELATED_PER_SYMBOL)
}

/** The symbols, in a changed symbol's own file and in the files that import it, that name it. */
function callersOf(graph: SourceGraph, path: string, symbol: SourceSymbol): readonly ContextSlice[] {
  return [path, ...graph.references(path, symbol.name)]
    .flatMap((caller) =>
      mentionLines(graph.files.get(caller)?.text ?? '', symbol.name).flatMap((line): ContextSlice[] => {
        const enclosing = graph.enclosing(caller, line)
        if (enclosing === undefined || (caller === path && enclosing.first === symbol.first)) return []
        return [{ first: enclosing.first, last: enclosing.last, of: symbol.name, path: caller, reason: 'caller', symbol: enclosing.name }]
      }),
    )
    .filter((slice, index, all) => all.findIndex((other) => other.path === slice.path && other.first === slice.first) === index)
    .slice(0, RELATED_PER_SYMBOL)
}

function around(line: number, context: number, lines: number): LineRange {
  return { first: Math.max(1, line - context), last: Math.max(1, Math.min(lines, line + context)) }
}

/**
 * The lines of a file's new text the change added or rewrote, from 1. A
 * removal with nothing added in its place touches the line after it (or
 * the last line). An added file is changed throughout.
 */
export function changedLines(before: string | undefined, after: string): readonly number[] {
  const next = linesOf(after)
  if (before === undefined) return next.map((_, i) => i + 1)
  const old = linesOf(before)
  let start = 0
  while (start < old.length && start < next.length && old[start] === next[start]) start += 1
  let end = 0
  while (end < old.length - start && end < next.length - start && old[old.length - 1 - end] === next[next.length - 1 - end]) end += 1
  const oldMiddle = old.slice(start, old.length - end)
  const newMiddle = next.slice(start, next.length - end)
  if (newMiddle.length === 0) return oldMiddle.length === 0 || next.length === 0 ? [] : [Math.min(start + 1, next.length)]
  if (oldMiddle.length * newMiddle.length > DIFF_CELLS) return newMiddle.map((_, i) => start + i + 1)
  const kept = commonLines(oldMiddle, newMiddle)
  const added = newMiddle.flatMap((_, i) => (kept.has(i) ? [] : [start + i + 1]))
  return added.length === 0 ? [Math.min(start + 1, next.length)] : added
}

/** A text's lines: a final newline ends the last line rather than starting another. */
function linesOf(text: string): string[] {
  return text === '' ? [] : text.replace(/\n$/, '').split('\n')
}

/** The indexes of `b` on a longest common subsequence of `a` and `b`. */
function commonLines(a: readonly string[], b: readonly string[]): ReadonlySet<number> {
  const width = b.length + 1
  const table = new Uint32Array((a.length + 1) * width)
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      table[i * width + j] =
        a[i] === b[j] ? (table[(i + 1) * width + j + 1] ?? 0) + 1 : Math.max(table[(i + 1) * width + j] ?? 0, table[i * width + j + 1] ?? 0)
    }
  }
  const kept = new Set<number>()
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      kept.add(j)
      i += 1
      j += 1
    } else if ((table[(i + 1) * width + j] ?? 0) >= (table[i * width + j + 1] ?? 0)) i += 1
    else j += 1
  }
  return kept
}
