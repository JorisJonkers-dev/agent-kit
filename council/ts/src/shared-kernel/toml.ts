/**
 * A small TOML reader for the TOML files the runners read: a Vibe critic agent
 * file, the home's `config.toml`, and the models that configuration declares.
 * It reads what those files hold (tables, arrays of tables, dotted and quoted
 * keys, basic and literal strings, single- and multi-line, booleans, numbers,
 * arrays and inline tables, comments) and throws on anything it cannot read,
 * so a caller never acts on half a file. Dates and the finer points of number
 * syntax are kept as their text.
 */
export type TomlTable = Record<string, unknown>

export class TomlError extends Error {}

export function readToml(text: string): TomlTable {
  return new TomlReader(text).document()
}

class TomlReader {
  private at = 0

  constructor(private readonly text: string) {}

  document(): TomlTable {
    const root: TomlTable = {}
    let table = root
    for (;;) {
      this.skip(true)
      if (this.at >= this.text.length) return root
      if (this.text.startsWith('[[', this.at)) {
        this.at += 2
        const path = this.keyPath(']]')
        const list = this.arrayAt(root, path)
        table = {}
        list.push(table)
      } else if (this.peek() === '[') {
        this.at += 1
        table = this.tableAt(root, this.keyPath(']'))
      } else {
        const path = this.keyPath('=')
        this.assign(table, path, this.value())
      }
      this.endOfLine()
    }
  }

  /** A dotted key, up to and past `end`. */
  private keyPath(end: string): readonly string[] {
    const path: string[] = []
    for (;;) {
      this.skip(false)
      path.push(this.key())
      this.skip(false)
      if (this.text.startsWith(end, this.at)) {
        this.at += end.length
        return path
      }
      this.expect('.')
    }
  }

  private key(): string {
    const char = this.peek()
    if (char === '"' || char === "'") return this.string()
    const bare = /^[A-Za-z0-9_-]+/.exec(this.text.slice(this.at))
    if (bare === null) throw this.error('a key')
    this.at += bare[0].length
    return bare[0]
  }

  private value(): unknown {
    this.skip(false)
    const char = this.peek()
    if (char === '"' || char === "'") return this.string()
    if (char === '[') return this.array()
    if (char === '{') return this.inlineTable()
    const word = /^[^\s,\]}#]+/.exec(this.text.slice(this.at))
    if (word === null) throw this.error('a value')
    this.at += word[0].length
    if (word[0] === 'true') return true
    if (word[0] === 'false') return false
    const number = Number(word[0].replace(/_/g, ''))
    return Number.isNaN(number) ? word[0] : number
  }

  private string(): string {
    const quote = this.peek()
    const triple = quote.repeat(3)
    const multi = this.text.startsWith(triple, this.at)
    const close = multi ? triple : quote
    this.at += close.length
    // A newline right after the opening of a multi-line string is not part of it.
    if (multi) this.at += /^\r?\n/.exec(this.text.slice(this.at))?.[0].length ?? 0
    let out = ''
    for (;;) {
      if (this.at >= this.text.length) throw this.error(`the closing ${close}`)
      if (this.text.startsWith(close, this.at)) {
        this.at += close.length
        return out
      }
      const char = this.text.charAt(this.at)
      if (char === '\n' && !multi) throw this.error(`the closing ${close}`)
      if (char === '\\' && quote === '"') {
        out += this.escape()
        continue
      }
      out += char
      this.at += 1
    }
  }

  private escape(): string {
    const code = this.text.charAt(this.at + 1)
    const simple: Readonly<Record<string, string>> = { '"': '"', '\\': '\\', b: '\b', f: '\f', n: '\n', r: '\r', t: '\t' }
    const plain = simple[code]
    if (plain !== undefined) {
      this.at += 2
      return plain
    }
    const unicode = /^u([0-9A-Fa-f]{4})|^U([0-9A-Fa-f]{8})/.exec(this.text.slice(this.at + 1))
    if (unicode === null) throw this.error('a valid escape')
    this.at += 1 + unicode[0].length
    return String.fromCodePoint(parseInt(unicode[1] ?? unicode[2] ?? '', 16))
  }

  private array(): unknown[] {
    this.at += 1
    const items: unknown[] = []
    for (;;) {
      this.skip(true)
      if (this.peek() === ']') {
        this.at += 1
        return items
      }
      items.push(this.value())
      this.skip(true)
      if (this.peek() !== ']') this.expect(',')
    }
  }

  private inlineTable(): TomlTable {
    this.at += 1
    const table: TomlTable = {}
    this.skip(false)
    if (this.peek() === '}') {
      this.at += 1
      return table
    }
    for (;;) {
      this.assign(table, this.keyPath('='), this.value())
      this.skip(false)
      if (this.peek() === '}') {
        this.at += 1
        return table
      }
      this.expect(',')
    }
  }

  private assign(table: TomlTable, path: readonly string[], value: unknown): void {
    const last = path.at(-1) ?? ''
    const parent = this.tableAt(table, path.slice(0, -1))
    if (Object.hasOwn(parent, last)) throw this.error(`a key not already defined (${last})`)
    parent[last] = value
  }

  /** The table at `path` under `root`, created where missing; under an array of tables, its latest table. */
  private tableAt(root: TomlTable, path: readonly string[]): TomlTable {
    let table = root
    for (const key of path) {
      const next = table[key] ?? (table[key] = {})
      const resolved = Array.isArray(next) ? (next.at(-1) as unknown) : next
      if (typeof resolved !== 'object' || resolved === null) throw this.error(`a table at ${key}`)
      table = resolved as TomlTable
    }
    return table
  }

  private arrayAt(root: TomlTable, path: readonly string[]): unknown[] {
    const parent = this.tableAt(root, path.slice(0, -1))
    const last = path.at(-1) ?? ''
    const list = parent[last] ?? (parent[last] = [])
    if (!Array.isArray(list)) throw this.error(`an array of tables at ${last}`)
    return list
  }

  /** Skips spaces and comments, and newlines too when `lines` is set. */
  private skip(lines: boolean): void {
    for (;;) {
      const char = this.peek()
      if (char === ' ' || char === '\t' || char === '\r' || (lines && char === '\n')) this.at += 1
      else if (char === '#') while (this.at < this.text.length && this.peek() !== '\n') this.at += 1
      else return
    }
  }

  private endOfLine(): void {
    this.skip(false)
    if (this.at < this.text.length) this.expect('\n')
  }

  private expect(char: string): void {
    if (this.peek() !== char) throw this.error(`'${char}'`)
    this.at += 1
  }

  private peek(): string {
    return this.text.charAt(this.at)
  }

  private error(wanted: string): TomlError {
    const line = this.text.slice(0, this.at).split('\n').length
    return new TomlError(`line ${String(line)}: expected ${wanted}`)
  }
}
