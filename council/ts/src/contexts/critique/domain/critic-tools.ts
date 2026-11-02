import type { SearchLimits, SearchToolName } from './packet-search.js'

/** How a client starts the runner's search server: a command line, its root among the arguments. */
export interface SearchServerLaunch {
  readonly command: string
  readonly args: readonly string[]
}

/**
 * The tools one critic has (ADR-0006), which its prompt names exactly.
 * A Vibe critic reads with `read_file`, and searches through the runner's
 * search server when the server proved it cannot see past the packet and
 * started on this critic's copy; otherwise it reads alone. An agy critic
 * reads with `view_file`, and searches with agy's own `list_dir`,
 * `grep_search` and `find_by_name` only once setup has proven agy
 * read-only on this machine (#178); until then it reads alone.
 */
export type CriticToolset =
  | { readonly kind: 'vibe-read' }
  | {
      readonly kind: 'vibe-search'
      readonly search: readonly SearchToolName[]
      readonly limits: SearchLimits
      readonly server: SearchServerLaunch
    }
  | { readonly kind: 'agy'; readonly search: boolean }

export const READ_ONLY_TOOLSET: CriticToolset = { kind: 'vibe-read' }

/** One tool call as a critic's journal records it: the tool, how it ended, and the error code when it failed. */
export interface ToolCall {
  readonly tool: string
  readonly status: string
  readonly code?: string
}

/** The name the runner's search server goes by in Vibe; its tools are `tools.mcp_packet.<tool>` there. */
export const VIBE_SEARCH_SERVER = 'packet'

/** agy's own read-only tools a critic is told about: one to read and three to list and search. */
export const AGY_TOOLS = ['view_file', 'list_dir', 'grep_search', 'find_by_name'] as const

/** The tool names the critic's prompt gives, in that order. */
export function toolNames(toolset: CriticToolset): readonly string[] {
  if (toolset.kind === 'agy') return toolset.search ? AGY_TOOLS : ['view_file']
  if (toolset.kind === 'vibe-read') return ['read_file']
  return ['read_file', 'run_typescript', ...toolset.search.map((tool) => `mcp_${VIBE_SEARCH_SERVER}.${tool}`)]
}

/** The tool a critic reads one file with. */
export function readTool(toolset: CriticToolset): string {
  return toolset.kind === 'agy' ? 'view_file' : 'read_file'
}

const SWITCHED_OFF = 'You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns.'

const SEARCH_FIRST =
  'Search before you read: a search answer costs far less than a whole file, so read a file in full only when you need more than the lines around a match.'

const SIGNATURES: Readonly<Record<SearchToolName, string>> = {
  grep: [
    '  /** Every line matching a JavaScript regular expression, as `path:line: text`; `path` limits the search to one file or directory, `glob` to matching paths. */',
    '  function grep(args: { pattern: string; path?: string; glob?: string; ignore_case?: boolean }): Promise<string>',
  ].join('\n'),
  list: [
    '  /** Every file path matching a glob, such as `tree/src/*.ts`, where `**` is any number of directories and `*` any characters but a slash. */',
    '  function list(args: { glob: string }): Promise<string>',
  ].join('\n'),
  symbol: [
    '  /** Where a top-level symbol of `tree/REPO-MAP.md` is defined, as `tree/<path>:<line>`, with its kind and signature. */',
    '  function symbol(args: { name: string }): Promise<string>',
  ].join('\n'),
}

/** The prompt's `# Tools` section: exactly the tools this critic has, and how to call them. */
export function toolsSection(toolset: CriticToolset): string {
  if (toolset.kind === 'vibe-read') {
    return [
      '# Tools',
      '',
      'You have one tool: `read_file`, which reads one file of your working directory, given its path as listed below. You cannot search: no search tool is switched on for you. ' +
        SWITCHED_OFF,
    ].join('\n')
  }
  if (toolset.kind === 'agy' && !toolset.search) {
    return [
      '# Tools',
      '',
      'You have one tool: `view_file`, which reads one file of your working directory, given its path as listed below. You cannot search: listing directories and searching files are switched off for you. ' +
        SWITCHED_OFF,
    ].join('\n')
  }
  if (toolset.kind === 'agy') {
    return [
      '# Tools',
      '',
      'Your tools read and search your working directory and see nothing outside it: `view_file` reads one file, `list_dir` lists a directory, `grep_search` searches the contents of files and `find_by_name` finds files by name. ' +
        SEARCH_FIRST,
      '',
      SWITCHED_OFF,
    ].join('\n')
  }
  const server = `mcp_${VIBE_SEARCH_SERVER}`
  return [
    '# Tools',
    '',
    `You have \`read_file\`, which reads one file of your working directory given its path, and these search functions over your working directory, which you call from \`run_typescript\` as \`tools.${server}.<function>\`:`,
    '',
    '```typescript',
    `declare namespace tools.${server} {`,
    ...toolset.search.map((tool) => SIGNATURES[tool]),
    '}',
    '```',
    '',
    `For example, call \`run_typescript\` with \`{"code": "async function main() {\\n  return await tools.${server}.grep({ pattern: 'TODO', path: 'tree' })\\n}"}\`.`,
    '',
    `The functions see nothing outside your working directory. An answer holds at most ${String(toolset.limits.max_matches)} matches and ${String(toolset.limits.max_bytes)} bytes; past that it ends by saying how many were left out, so search narrower rather than wider. ${SEARCH_FIRST}`,
    '',
    SWITCHED_OFF,
  ].join('\n')
}

/**
 * What setup's read-only proof has a Vibe critic try through `run_typescript`
 * (ADR-0006, amended): a shell command, a file write and a process start,
 * each of which must come back `tool_denied`, and one search, which must
 * complete. The search shows the gateway works; the refusals show it reaches
 * nothing else.
 */
export const GATEWAY_PROOF = {
  code: [
    'async function main() {',
    '  return await Promise.allSettled([',
    "    tools.file_system.bash({ command: 'echo probe > probe.txt' }),",
    "    tools.file_system.write_file({ path: 'probe.txt', content: 'probe' }),",
    "    tools.process.start({ command: 'echo probe' }),",
    `    tools.mcp_${VIBE_SEARCH_SERVER}.grep({ pattern: 'probe' }),`,
    '  ])',
    '}',
  ].join('\n'),
  refused: ['file_system.bash', 'file_system.write_file', 'process.start'],
  searched: `mcp_${VIBE_SEARCH_SERVER}.grep`,
} as const

/** Why the critic's tool calls do not show the gateway refusing all but search; none when they do. */
export function gatewayProblems(calls: readonly ToolCall[]): readonly string[] {
  const problems = GATEWAY_PROOF.refused.flatMap((tool) => {
    const call = calls.find((entry) => entry.tool === tool)
    if (call === undefined) return [`${tool} was not tried, so its refusal is not shown`]
    if (call.status === 'failed' && call.code === 'tool_denied') return []
    return [`${tool} came back ${call.status}${call.code === undefined ? '' : ` (${call.code})`}, not tool_denied`]
  })
  const searched = calls.some((entry) => entry.tool === GATEWAY_PROOF.searched && entry.status === 'completed')
  return searched ? problems : [...problems, `${GATEWAY_PROOF.searched} did not complete through run_typescript`]
}
