import { JsonFields } from './json.js'
import { REPO_MAP_PATH } from './repo-map.js'

/**
 * How much one search answer may hold (ADR-0006): at most `max_matches`
 * matches and `max_bytes` bytes of text. Both live in routing, under
 * `search`, so the live checks can tune them.
 */
export interface SearchLimits {
  readonly max_matches: number
  readonly max_bytes: number
}

export const DEFAULT_SEARCH_LIMITS: SearchLimits = { max_bytes: 8192, max_matches: 50 }

export function readSearchLimits(fields: JsonFields): SearchLimits {
  return { max_bytes: fields.positiveInteger('max_bytes'), max_matches: fields.positiveInteger('max_matches') }
}

/** The three tools of the runner's search server. */
export const SEARCH_TOOLS = ['grep', 'list', 'symbol'] as const
export type SearchToolName = (typeof SEARCH_TOOLS)[number]

/** `symbol` looks names up in the repo map, so it is offered only when the packet holds one. */
export function offeredSearchTools(hasRepoMap: boolean): readonly SearchToolName[] {
  return hasRepoMap ? SEARCH_TOOLS : ['grep', 'list']
}

export interface SearchToolDefinition {
  readonly name: SearchToolName
  readonly description: string
  readonly inputSchema: Readonly<Record<string, unknown>>
}

const PATH_NOTE = 'Paths are relative to the packet root; nothing outside the packet is reachable.'

export const SEARCH_TOOL_DEFINITIONS: Readonly<Record<SearchToolName, SearchToolDefinition>> = {
  grep: {
    description: `Searches the contents of the packet's files for a JavaScript regular expression and returns each matching line as \`path:line: text\`. ${PATH_NOTE}`,
    inputSchema: {
      additionalProperties: false,
      properties: {
        glob: { description: 'Only search files whose path matches this glob, such as `tree/**/*.ts`.', type: 'string' },
        ignore_case: { description: 'Match without regard to case.', type: 'boolean' },
        path: { description: 'Only search this file or directory, such as `tree/src`.', type: 'string' },
        pattern: { description: 'A JavaScript regular expression, such as `parseRouting\\(`.', type: 'string' },
      },
      required: ['pattern'],
      type: 'object',
    },
    name: 'grep',
  },
  list: {
    description: `Lists the packet's file paths that match a glob: \`**\` is any number of directories, \`*\` any characters but \`/\`, \`{a,b}\` either. ${PATH_NOTE}`,
    inputSchema: {
      additionalProperties: false,
      properties: { glob: { description: 'The glob, such as `tree/**/*.md`; `**` lists every file.', type: 'string' } },
      required: ['glob'],
      type: 'object',
    },
    name: 'list',
  },
  symbol: {
    description: `Looks a top-level symbol up in \`${REPO_MAP_PATH}\` and returns where each match is defined, as \`tree/<path>:<line>\`, with its kind, signature and reference count.`,
    inputSchema: {
      additionalProperties: false,
      properties: { name: { description: 'The symbol name; an exact match first, else every name containing it, ignoring case.', type: 'string' } },
      required: ['name'],
      type: 'object',
    },
    name: 'symbol',
  },
}

/** The noun a capped answer counts in. */
type Unit = 'matches' | 'paths'

/**
 * The answer for these result lines, cut to the limits: the first lines
 * that fit both caps, then a marker that says how many were left out and
 * asks for a narrower search. `total` counts every result, kept or not.
 */
export function cappedAnswer(lines: readonly string[], limits: SearchLimits, unit: Unit, empty: string): string {
  if (lines.length === 0) return empty
  const kept: string[] = []
  let size = 0
  for (const line of lines.slice(0, limits.max_matches)) {
    const bytes = new TextEncoder().encode(`${line}\n`).length
    if (size + bytes > limits.max_bytes) break
    kept.push(line)
    size += bytes
  }
  const left = lines.length - kept.length
  if (left === 0) return kept.join('\n')
  return [...kept, `[${String(left)} more ${unit} left out; narrow the pattern, path or glob]`].join('\n')
}

/** A search line holds at most this many characters of the matched line, so one long line cannot fill an answer. */
export const LINE_MAX_CHARS = 300

export function clipLine(text: string): string {
  return text.length <= LINE_MAX_CHARS ? text : `${text.slice(0, LINE_MAX_CHARS)}…`
}

interface MapSymbol {
  readonly path: string
  readonly name: string
  readonly line: string
  readonly text: string
}

/**
 * The repo map's symbols named `name`: an exact match first, else every
 * name that contains it, ignoring case. Each reads
 * `tree/<path>:<line>: <kind> <name>, <refs> refs: <signature>`.
 */
export function symbolMatches(repoMap: string, name: string): readonly string[] {
  const symbols: MapSymbol[] = []
  let path: string | undefined
  for (const line of repoMap.split('\n')) {
    const heading = /^## `(.+)` \(referenced by/.exec(line)
    if (heading !== null) {
      path = heading[1]
      continue
    }
    const symbol = /^- `([^`]+)` \(([^,]+), line (\d+), (\d+) refs\): `(.*)`$/.exec(line)
    if (symbol === null || path === undefined) continue
    const [, symbolName = '', kind = '', at = '', refs = '', signature = ''] = symbol
    symbols.push({ line: at, name: symbolName, path, text: `${kind} ${symbolName}, ${refs} refs: ${signature}` })
  }
  const exact = symbols.filter((symbol) => symbol.name === name)
  const lower = name.toLowerCase()
  const found = exact.length > 0 ? exact : symbols.filter((symbol) => symbol.name.toLowerCase().includes(lower))
  return found.map((symbol) => `tree/${symbol.path}:${symbol.line}: ${clipLine(symbol.text)}`)
}

/**
 * What setup and fan-out ask a search server to prove that it cannot see
 * past its packet: a packet with one file inside, a file outside it, and
 * two symlinks out of it, one to the outside directory and one to the file.
 */
export const REACH_PROOF = {
  inside: { path: 'inside.md', text: 'reach proof: inside the packet\n' },
  marker: 'reach proof',
  outside: { dir: 'outside', path: 'outside/secret.md', text: 'reach proof: OUTSIDE-THE-PACKET\n' },
  outsideMarker: 'OUTSIDE-THE-PACKET',
  symlinks: [
    { path: 'escape', target: '../outside' },
    { path: 'escape.md', target: '../outside/secret.md' },
  ],
} as const

/** One call a client makes to a search server. */
export interface SearchCall {
  readonly tool: SearchToolName
  readonly arguments: Readonly<Record<string, unknown>>
}

/**
 * The calls of the reach proof: every way out a critic could try (`..`, an
 * absolute path, a symlink to a directory and one to a file, a glob that
 * climbs) and one search that must find the inside file. `outsideDir` is
 * the outside directory's absolute path.
 */
export function reachProofCalls(outsideDir: string): readonly SearchCall[] {
  const grep = (args: Readonly<Record<string, unknown>>): SearchCall => ({ arguments: { pattern: REACH_PROOF.marker, ...args }, tool: 'grep' })
  return [
    grep({}),
    grep({ path: '../outside' }),
    grep({ path: outsideDir }),
    grep({ path: 'escape' }),
    grep({ path: 'escape.md' }),
    grep({ glob: '../**' }),
    { arguments: { glob: '**' }, tool: 'list' },
    { arguments: { glob: '../**' }, tool: 'list' },
    { arguments: { glob: `${outsideDir}/**` }, tool: 'list' },
  ]
}

/**
 * Why the proof's answers show a server that reaches past its packet, or
 * cannot search it: none when the first answer finds the inside file and
 * no answer names the outside file or quotes it.
 */
export function reachProblems(answers: readonly string[]): readonly string[] {
  const problems: string[] = []
  if (!(answers[0] ?? '').includes(REACH_PROOF.inside.path)) problems.push('a search of the whole packet did not find the file inside it')
  answers.forEach((answer, index) => {
    if (answer.includes(REACH_PROOF.outsideMarker) || answer.includes('secret.md')) {
      problems.push(`call ${String(index + 1)} returned text from outside the packet`)
    }
  })
  return problems
}
