import type { ProcessCommand } from '../../../ports/index.js'
import type { AttachedServer, ExtraName, SandboxProfile } from '../domain/extras.js'
import { agyOutput, claudeOutput, openCodeOutput, vibeOutput, type ClientOutput } from '../domain/outputs.js'
import type { RunClient } from '../domain/run.js'

/** The mark the gateway puts in every delegate's environment, and refuses to run under. */
export const DELEGATION_MARK = 'AGENT_KIT_DELEGATION'

/** The Vibe agent a read-only delegation runs through: read tools only, no subagents, nothing outside its workdir. */
export const READ_ONLY_AGENT = 'delegate-readonly'

/** Seconds the outer timer allows beyond a client's own time limit, so the client's own stop is the one that reports. */
const OUTER_TIMEOUT_GRACE_S = 60

/**
 * What a Vibe run's environment holds: the isolation Vibe reads from `VIBE_*`
 * variables, which sits above the home's `config.toml` and below the agent
 * file.
 */
const VIBE_ENV: Readonly<Record<string, string>> = {
  VIBE_ENABLE_MCP: 'false',
  VIBE_ENABLE_SUBAGENTS: 'false',
  VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
  VIBE_OUTSIDE_WORKDIR: 'deny',
}

/**
 * The variables a Vibe launch unsets, so a run started from a `vibe-study` or
 * `vibe-private` session neither spends on the Study account nor runs in the
 * private home.
 */
const VIBE_UNSET = ['-u', 'MISTRAL_API_KEY', '-u', 'VIBE_HOME'] as const

/**
 * The variables a Vibe launch sets itself: the isolation above, the servers it
 * attaches, and the two it unsets. A caller's `extras.env` may not name any of
 * them, so the launcher's own isolation always wins.
 */
const VIBE_PROTECTED_ENV = [
  'MISTRAL_API_KEY',
  'VIBE_ENABLE_MCP',
  'VIBE_ENABLE_SUBAGENTS',
  'VIBE_HOME',
  'VIBE_INCLUDE_PROJECT_CONTEXT',
  'VIBE_MCP_SERVERS',
  'VIBE_OUTSIDE_WORKDIR',
] as const

/** How long a client's own MCP server may take to start, and one of its tools to answer. */
const MCP_STARTUP_TIMEOUT_S = 10
const MCP_TOOL_TIMEOUT_S = 30

/**
 * agy's own tool families, in the order its deny list names them. A read-only
 * run denies every family the caller did not name among the tools it may use.
 */
const AGY_TOOL_FAMILIES = ['mcp', 'read_url', 'execute_url', 'command', 'write_file', 'list_dir', 'grep_search', 'find_by_name'] as const

/** Where one launch reads its prompt and writes what Vibe exports, and the directory the client works in. */
export interface LaunchPaths {
  readonly exportDir: string
  readonly promptPath: string
  /** The directory the client runs in: the repository, a throwaway worktree of it, or an empty directory. */
  readonly workDir: string
}

/** What one launch needs of its input: the model, and the limits the client enforces itself. */
export interface LaunchRequest {
  readonly model: string
  readonly time_limit_s: number
  readonly max_price_usd?: number
}

/** What a caller adds to one launch, with the client's home servers already resolved. */
export interface LaunchExtras {
  readonly mcp_servers: readonly AttachedServer[]
  /** The MCP servers the client's home declares, which a run with `disable_home_mcp` switches off by name. */
  readonly homeServers: readonly string[]
  readonly sandbox?: SandboxProfile
  readonly agent?: string
  readonly tools: readonly string[]
  readonly schema?: string
  readonly env: Readonly<Record<string, string>>
}

/** How the gateway starts one client and reads what it printed. */
export interface ClientLauncher {
  /** Whether the client writes an `export.json` beside its output, which carries its tokens and dollars. */
  readonly writesExport: boolean
  /** The extras this client can apply; one it cannot apply is refused rather than dropped. */
  readonly extras: readonly ExtraName[]
  /** The variables this launcher sets itself, which a caller's `extras.env` may not name. */
  readonly protectedEnv: readonly string[]
  /** The client's non-interactive launch for one read-only run; `prompt` is the wrapped prompt text. */
  command(binary: string, input: LaunchRequest, paths: LaunchPaths, prompt: string, extras?: LaunchExtras): ProcessCommand
  /** What the client's output (and its export, where it writes one) says about the run. */
  read(stdout: string, exported: unknown): ClientOutput
}

/**
 * The MCP servers a Vibe launch configures: the caller's, in its packet, and
 * each of the home's switched off by name. Vibe joins the servers of every
 * layer by name, so a home server the caller attaches itself is replaced by
 * the caller's, and every other one is replaced with a switched-off entry
 * that never starts.
 */
function vibeServers(extras: LaunchExtras, workDir: string): readonly Readonly<Record<string, unknown>>[] {
  const own = extras.mcp_servers.map((server) => ({
    args: [...server.args],
    command: server.command,
    cwd: workDir,
    name: server.name,
    startup_timeout_sec: MCP_STARTUP_TIMEOUT_S,
    tool_timeout_sec: MCP_TOOL_TIMEOUT_S,
    transport: 'stdio',
  }))
  const off = extras.homeServers
    .filter((name) => !extras.mcp_servers.some((server) => server.name === name))
    .map((name) => ({ command: 'false', disabled: true, name, transport: 'stdio' }))
  return [...own, ...off]
}

/** A Vibe run's environment: the caller's own variables, then its isolation and the servers it may reach, which win. */
function vibeEnv(extras: LaunchExtras | undefined, workDir: string): Readonly<Record<string, string>> {
  const servers = extras === undefined ? [] : vibeServers(extras, workDir)
  return {
    ...extras?.env,
    ...VIBE_ENV,
    ...(servers.length === 0 ? {} : { VIBE_ENABLE_MCP: 'true', VIBE_MCP_SERVERS: JSON.stringify(servers) }),
  }
}

/**
 * Vibe: the wrapped prompt from a file, the client model by the alias `models`
 * lists, the read-only agent, the tools the caller allows, and Vibe's own time
 * and price limits. The model is always named, so a run whose agent file names
 * another one still runs the model the policy checked and the ledger records.
 * Every launch is the read-only profile: `MISTRAL_API_KEY` and `VIBE_HOME`
 * unset, MCP, subagents and the project's `AGENTS.md` pointer off, and nothing
 * outside the workdir.
 */
const vibe: ClientLauncher = {
  command: (binary, input, paths, _prompt, extras) => ({
    args: [
      ...VIBE_UNSET,
      binary,
      '--prompt-file',
      paths.promptPath,
      '--model',
      input.model,
      '--agent',
      extras?.agent ?? READ_ONLY_AGENT,
      '--trust',
      '--workdir',
      paths.workDir,
      '--output',
      'json',
      ...(extras?.tools ?? []).flatMap((tool) => ['--enabled-tools', tool]),
      '--time-limit',
      String(input.time_limit_s),
      ...(input.max_price_usd === undefined ? [] : ['--max-price', String(input.max_price_usd)]),
      '--output-dir',
      paths.exportDir,
    ],
    command: 'env',
    cwd: paths.workDir,
    env: vibeEnv(extras, paths.workDir),
    timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }),
  extras: ['agent', 'disable_home_mcp', 'env', 'mcp_servers', 'tools'],
  protectedEnv: VIBE_PROTECTED_ENV,
  read: (stdout, exported) => vibeOutput(stdout, exported),
  writesExport: true,
}

/** The reasoning effort a model id names by its last word (`gemini-3.8-flash-high`); medium for an id that names none. */
function effortOf(model: string): string {
  return /-(low|medium|high)$/.exec(model)?.[1] ?? 'medium'
}

/** The session settings a read-only agy run gets: every tool family the caller did not name is denied. */
function readOnlySettings(tools: readonly string[]): unknown {
  return {
    permissions: { deny: AGY_TOOL_FAMILIES.filter((family) => !tools.includes(family)).map((family) => `${family}(*)`) },
  }
}

/**
 * Antigravity. A run with the read-only sandbox profile prints a stream in a
 * fresh sandboxed session, with slash commands off, the schema the caller
 * gives and every tool family it did not name denied. A run without it prints
 * one JSON object in plan mode, with the effort its model id names. Its stdin
 * is closed either way.
 */
const agy: ClientLauncher = {
  command: (binary, input, paths, prompt, extras) => ({
    args:
      extras?.sandbox === 'read-only'
        ? [
            '-p',
            prompt,
            '--output-format',
            'stream-json',
            '--model',
            input.model,
            '--print-timeout',
            `${String(input.time_limit_s)}s`,
            '--disable-slash-commands',
            '--sandbox',
            ...(extras.schema === undefined ? [] : ['--json-schema', extras.schema]),
            '--settings',
            JSON.stringify(readOnlySettings(extras.tools)),
          ]
        : [
            '--print',
            prompt,
            '--model',
            input.model,
            '--output-format',
            'json',
            '--effort',
            effortOf(input.model),
            '--mode',
            'plan',
            '--print-timeout',
            `${String(input.time_limit_s)}s`,
          ],
    command: binary,
    cwd: paths.workDir,
    env: extras?.env ?? {},
    timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }),
  extras: ['env', 'sandbox', 'schema', 'tools'],
  protectedEnv: [],
  read: (stdout) => agyOutput(stdout),
  writesExport: false,
}

/**
 * Claude Code: headless print mode with the prompt on stdin, in plan mode,
 * and the read tools alone. It has no time limit of its own, so the outer
 * timer is the limit.
 */
const claude: ClientLauncher = {
  command: (binary, input, paths, prompt, extras) => ({
    args: ['-p', '--model', input.model, '--output-format', 'json', '--permission-mode', 'plan', '--allowedTools', 'Read,Grep,Glob'],
    command: binary,
    cwd: paths.workDir,
    env: extras?.env ?? {},
    stdin: prompt,
    timeoutMs: input.time_limit_s * 1000,
  }),
  extras: ['env'],
  protectedEnv: [],
  read: (stdout) => claudeOutput(stdout),
  writesExport: false,
}

/**
 * OpenCode: `opencode run` with the provider/model, JSON events and the
 * prompt as its message. It works in the directory it is started in, and has
 * no time limit of its own, so the outer timer is the limit.
 */
const opencode: ClientLauncher = {
  command: (binary, input, paths, prompt, extras) => ({
    args: ['run', '--model', input.model, '--format', 'json', prompt],
    command: binary,
    cwd: paths.workDir,
    env: extras?.env ?? {},
    timeoutMs: input.time_limit_s * 1000,
  }),
  extras: ['env'],
  protectedEnv: [],
  read: (stdout) => openCodeOutput(stdout),
  writesExport: false,
}

/** The launcher of every client a run can be handed to. */
export const LAUNCHERS: Readonly<Record<RunClient, ClientLauncher>> = { agy, claude, opencode, vibe }
