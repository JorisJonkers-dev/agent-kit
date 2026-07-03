import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

interface ScalarSpan {
  readonly value: string
  readonly start: number
  readonly end: number
}

interface MappingContext {
  readonly indent: number
  path?: string
  sha256Line?: number
  sha256Span?: ScalarSpan
  emitted?: boolean
}

interface PinnedPathEntry {
  readonly path: string
  readonly sha256Line: number
  readonly sha256Span: ScalarSpan
}

const KEY_RE = /^(\s*)(?:-\s*)?([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/
const HASH_RE = /^[a-f0-9]{64}$/i

function parseScalar(raw: string, line: string): ScalarSpan | undefined {
  const offset = line.length - raw.length
  const trimmedStart = raw.search(/\S/)
  if (trimmedStart === -1) {
    return undefined
  }
  const start = offset + trimmedStart
  const trimmed = raw.slice(trimmedStart)

  if (trimmed.startsWith('"')) {
    let escaped = false
    for (let i = 1; i < trimmed.length; i += 1) {
      const ch = trimmed.charAt(i)
      if (escaped) {
        escaped = false
      } else if (ch === '\\') {
        escaped = true
      } else if (ch === '"') {
        const body = trimmed.slice(1, i)
        const value: unknown = JSON.parse(`"${body}"`)
        if (typeof value !== 'string') {
          return undefined
        }
        return { value, start: start + 1, end: start + i }
      }
    }
    return undefined
  }

  if (trimmed.startsWith("'")) {
    let body = ''
    for (let i = 1; i < trimmed.length; i += 1) {
      const ch = trimmed.charAt(i)
      if (ch === "'") {
        if (trimmed[i + 1] === "'") {
          body += "'"
          i += 1
          continue
        }
        return { value: body, start: start + 1, end: start + i }
      }
      body += ch
    }
    return undefined
  }

  const comment = trimmed.search(/\s+#/)
  const valueText = (comment === -1 ? trimmed : trimmed.slice(0, comment)).trimEnd()
  if (!valueText || valueText === '|' || valueText === '>') {
    return undefined
  }
  return { value: valueText, start, end: start + valueText.length }
}

function pushIfComplete(entries: PinnedPathEntry[], context: MappingContext): void {
  if (
    context.path !== undefined &&
    context.sha256Line !== undefined &&
    context.sha256Span !== undefined &&
    !context.emitted
  ) {
    entries.push({
      path: context.path,
      sha256Line: context.sha256Line,
      sha256Span: context.sha256Span,
    })
    context.emitted = true
  }
}

export function pinnedPathEntries(manifestText: string): readonly PinnedPathEntry[] {
  const entries: PinnedPathEntry[] = []
  const contexts: MappingContext[] = []
  const lines = manifestText.split('\n')

  lines.forEach((line, index) => {
    const match = KEY_RE.exec(line)
    if (!match) {
      return
    }

    const leading = match[1] ?? ''
    const hasDash = line.slice(leading.length).startsWith('-')
    const key = match[2]
    const raw = match[3] ?? ''
    const indent = leading.length + (hasDash ? 2 : 0)

    popContextsWhile(contexts, (context) => context.indent > indent)
    if (hasDash) {
      popContextsWhile(contexts, (context) => context.indent >= indent)
    }

    let context = contexts.findLast((item) => item.indent === indent)
    if (context === undefined) {
      context = { indent }
      contexts.push(context)
    }

    const scalar = parseScalar(raw, line)
    if (key === 'path' && scalar !== undefined) {
      context.path = scalar.value
      pushIfComplete(entries, context)
    } else if (key === 'sha256' && scalar !== undefined) {
      context.sha256Line = index
      context.sha256Span = scalar
      pushIfComplete(entries, context)
    }
  })

  return entries
}

function popContextsWhile(
  contexts: MappingContext[],
  shouldPop: (context: MappingContext) => boolean,
): void {
  let context = contexts.at(-1)
  while (context !== undefined && shouldPop(context)) {
    contexts.pop()
    context = contexts.at(-1)
  }
}

function sha256(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

export function updateManifestHashes(manifestPath: string, rootDir = resolve(manifestPath, '..')): number {
  const original = readFileSync(manifestPath, 'utf8')
  const entries = pinnedPathEntries(original)
  const lines = original.split('\n')
  let changed = 0

  for (const entry of entries) {
    const digest = sha256(resolve(rootDir, entry.path))
    if (!HASH_RE.test(entry.sha256Span.value) || entry.sha256Span.value !== digest) {
      const line = lines[entry.sha256Line]
      if (line === undefined) {
        throw new Error(`internal error: missing manifest line for ${entry.path}`)
      }
      lines[entry.sha256Line] =
        line.slice(0, entry.sha256Span.start) + digest + line.slice(entry.sha256Span.end)
      changed += 1
    }
  }

  if (changed > 0) {
    writeFileSync(manifestPath, lines.join('\n'))
  }
  return changed
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === process.argv[1]) {
  const manifestPath = resolve(process.argv[2] ?? resolve(import.meta.dirname, '../../manifest.yaml'))
  const rootDir = resolve(process.argv[3] ?? resolve(manifestPath, '..'))
  const changed = updateManifestHashes(manifestPath, rootDir)
  console.log(`updated ${String(changed)} manifest sha256 entr${changed === 1 ? 'y' : 'ies'}`)
}
