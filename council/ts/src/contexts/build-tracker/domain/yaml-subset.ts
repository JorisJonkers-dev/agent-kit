import { BuildTrackerError } from './state.js'

/**
 * A reader for the block-style YAML subset a ticket file uses: mappings,
 * block lists (indented under their key or level with it), and scalars that
 * are plain, single-quoted, double-quoted or `[]`. It reads nothing else:
 * anchors, flow collections with items, block scalars and multi-line scalars
 * are refused, so a file this reader accepts means to a full YAML reader
 * what it means here.
 */
export type YamlValue = string | number | boolean | null | readonly YamlValue[] | YamlMap

export interface YamlMap {
  readonly [key: string]: YamlValue
}

interface Line {
  indent: number
  text: string
  /** One-based, for messages. */
  no: number
}

const KEY = /^([A-Za-z_][\w-]*):(?: (.*))?$/
const INTEGER = /^-?\d+$/

export function parseYamlSubset(source: string): YamlValue {
  const lines: Line[] = []
  source.replace(/^\uFEFF/, '').split(/\r?\n/).forEach((raw, index) => {
    const body = raw.trimStart()
    if (body === '' || body.startsWith('#')) return
    if (/^ *\t/.test(raw)) throw refused(index + 1, 'a tab cannot indent')
    lines.push({ indent: raw.length - body.length, no: index + 1, text: body.trimEnd() })
  })
  const [first] = lines
  if (first === undefined) throw refused(1, 'the file is empty')
  const reader = new Reader(lines)
  const value = reader.node(first)
  const rest = reader.peek()
  if (rest !== undefined) throw refused(rest.no, 'this line does not fit its parent')
  return value
}

class Reader {
  private position = 0

  constructor(private readonly lines: Line[]) {}

  peek(): Line | undefined {
    return this.lines[this.position]
  }

  /** The block that starts at `line`, the next line, whose indentation is its own. */
  node(line: Line): YamlValue {
    return isItem(line.text) ? this.list(line.indent) : this.map(line.indent)
  }

  private list(indent: number): YamlValue {
    const items: YamlValue[] = []
    for (let line = this.peek(); line?.indent === indent && isItem(line.text); line = this.peek()) {
      const rest = line.text.slice(1).trimStart()
      if (rest === '') {
        this.position += 1
        const next = this.peek()
        if (next === undefined || next.indent <= indent) throw refused(line.no, 'a list item has no value')
        items.push(this.node(next))
      } else if (KEY.test(rest)) {
        const inner = indent + (line.text.length - rest.length)
        this.lines[this.position] = { indent: inner, no: line.no, text: rest }
        items.push(this.map(inner))
      } else {
        this.position += 1
        items.push(scalar(rest, line.no))
      }
    }
    return items
  }

  private map(indent: number): YamlValue {
    const entries: Record<string, YamlValue> = {}
    for (let line = this.peek(); line?.indent === indent; line = this.peek()) {
      if (isItem(line.text)) break
      const match = KEY.exec(line.text)
      if (match === null) throw refused(line.no, 'expected `key: value`')
      const key = String(match[1])
      if (key === '__proto__' || Object.hasOwn(entries, key)) throw refused(line.no, `the key ${key} appears twice`)
      this.position += 1
      const rest = (match[2] ?? '').replace(/^#.*$/, '').trim()
      if (rest !== '') {
        entries[key] = scalar(rest, line.no)
        continue
      }
      const next = this.peek()
      const nested = next !== undefined && (next.indent > indent || (next.indent === indent && isItem(next.text)))
      entries[key] = nested ? this.node(next) : null
    }
    const stray = this.peek()
    if (stray !== undefined && stray.indent > indent) throw refused(stray.no, 'unexpected indentation')
    return entries
  }
}

function isItem(text: string): boolean {
  return text === '-' || text.startsWith('- ')
}

function scalar(text: string, no: number): YamlValue {
  if (text.startsWith("'")) {
    const end = closingQuote(text)
    if (end < 0 || !/^\s*(?:#.*)?$/.test(text.slice(end + 1))) throw refused(no, 'a single-quoted value must end with its quote')
    return text.slice(1, end).replaceAll("''", "'")
  }
  if (text.startsWith('"')) {
    try {
      const value: unknown = JSON.parse(text.replace(/^("(?:[^"\\]|\\.)*")\s*(?:#.*)?$/, '$1'))
      if (typeof value === 'string') return value
    } catch {
      // Falls through to the refusal.
    }
    throw refused(no, 'a double-quoted value must be a JSON string')
  }
  if (text === '[]') return []
  if (/^[[{|>&*!%@`]/.test(text)) throw refused(no, 'this YAML form is not supported in a ticket file')
  const plain = text.replace(/\s+#.*$/, '')
  if (/: |:$/.test(plain)) throw refused(no, 'a plain value cannot hold `: `; quote it')
  if (plain === 'null' || plain === '~') return null
  if (plain === 'true') return true
  if (plain === 'false') return false
  return INTEGER.test(plain) ? Number(plain) : plain
}

/** The index of the quote that closes a single-quoted scalar, where `''` is an escaped quote. */
function closingQuote(text: string): number {
  for (let index = 1; index < text.length; index += 1) {
    if (text[index] !== "'") continue
    if (text[index + 1] === "'") index += 1
    else return index
  }
  return -1
}

function refused(no: number, reason: string): BuildTrackerError {
  return new BuildTrackerError('invalid-tickets', `the ticket file is not readable at line ${String(no)}: ${reason}`)
}

export function isYamlObject(value: YamlValue | undefined): value is YamlMap {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
