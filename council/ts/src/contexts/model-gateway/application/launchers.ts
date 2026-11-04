import { join } from 'node:path'

import type { ProcessCommand } from '../../../ports/index.js'
import { agyOutput, claudeOutput, hermesOutput, openCodeOutput, vibeOutput, type ClientOutput } from '../domain/outputs.js'
import type { RunClient, RunInput } from '../domain/run.js'

/** The mark the gateway puts in every delegate's environment, and refuses to run under. */
export const DELEGATION_MARK = 'AGENT_KIT_DELEGATION'

/** The Vibe agent a read-only delegation runs through: read tools only, no subagents, nothing outside its workdir. */
export const READ_ONLY_AGENT = 'delegate-readonly'

/** The Vibe agent a writing delegation runs through: it reads, searches, writes and edits files, and runs no shell. */
export const WRITE_AGENT = 'delegate-write'

/** Seconds the outer timer allows beyond a client's own time limit, so the client's own stop is the one that reports. */
const OUTER_TIMEOUT_GRACE_S = 60

/** What every delegate's environment holds: the mark that stops it delegating on. */
const MARKED_ENV: Readonly<Record<string, string>> = { [DELEGATION_MARK]: '1' }

/**
 * What a Vibe delegate's environment holds beyond the mark: the isolation Vibe
 * reads from `VIBE_*` variables, which sits above the home's `config.toml` and
 * below the agent file.
 */
const VIBE_ENV: Readonly<Record<string, string>> = {
  ...MARKED_ENV,
  VIBE_ENABLE_MCP: 'false',
  VIBE_ENABLE_SUBAGENTS: 'false',
  VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
  VIBE_OUTSIDE_WORKDIR: 'deny',
}

/** Where one launch reads its prompt and writes what the client exports, and the directory the client works in. */
export interface LaunchPaths {
  readonly exportDir: string
  readonly promptPath: string
  /** The run's own directory, where a client that writes a report of its own (Hermes' usage) puts it. */
  readonly runDir: string
  /** The directory the client runs in: the repository, a throwaway worktree of it, or an empty directory. */
  readonly workDir: string
}

/** How the gateway starts one client and reads what it printed. */
export interface ClientLauncher {
  /** The client's non-interactive launch for one run; `prompt` is the wrapped prompt text. */
  command(binary: string, input: RunInput, paths: LaunchPaths, prompt: string): ProcessCommand
  /** What the client's output (and the report it wrote, where it writes one) says about the run. */
  read(stdout: string, written: unknown): ClientOutput
  /**
   * The file the client writes beside its output that `read` takes as its
   * second argument: Vibe's `export.json`, Hermes' usage report. Undefined for
   * a client that writes none.
   */
  reportPath?(paths: LaunchPaths): string
}

/**
 * Vibe: the wrapped prompt from a file, the client model by the alias
 * `models` lists, the read-only agent, and Vibe's own time and price limits.
 */
const vibe: ClientLauncher = {
  command: (binary, input, paths) => ({
    args: [
      '--prompt-file',
      paths.promptPath,
      '--model',
      input.model,
      '--agent',
      input.mode === 'write' ? WRITE_AGENT : READ_ONLY_AGENT,
      '--trust',
      '--workdir',
      paths.workDir,
      '--output',
      'json',
      '--time-limit',
      String(input.time_limit_s),
      ...(input.max_price_usd === undefined ? [] : ['--max-price', String(input.max_price_usd)]),
      '--output-dir',
      paths.exportDir,
    ],
    command: binary,
    cwd: paths.workDir,
    env: VIBE_ENV,
    timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }),
  read: (stdout, exported) => vibeOutput(stdout, exported),
  reportPath: (paths) => join(paths.exportDir, 'export.json'),
}

/** The reasoning effort a model id names by its last word (`gemini-3.8-flash-high`); medium for an id that names none. */
function effortOf(model: string): string {
  return /-(low|medium|high)$/.exec(model)?.[1] ?? 'medium'
}

/**
 * Antigravity: print mode in plan mode (accept-edits for a writing run), with the model, the effort its id
 * names, JSON output, and its own print timeout. Its stdin is closed.
 */
const agy: ClientLauncher = {
  command: (binary, input, paths, prompt) => ({
    args: [
      '--print',
      prompt,
      '--model',
      input.model,
      '--output-format',
      'json',
      '--effort',
      effortOf(input.model),
      '--mode',
      input.mode === 'write' ? 'accept-edits' : 'plan',
      '--print-timeout',
      `${String(input.time_limit_s)}s`,
    ],
    command: binary,
    cwd: paths.workDir,
    env: MARKED_ENV,
    timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }),
  read: (stdout) => agyOutput(stdout),
}

/**
 * Claude Code: headless print mode with the prompt on stdin, in plan mode and
 * with the read tools alone, or for a writing run in accept-edits mode with
 * the edit tools besides. It has no time limit of its own, so the outer
 * timer is the limit.
 */
const claude: ClientLauncher = {
  command: (binary, input, paths, prompt) => ({
    args: [
      '-p',
      '--model',
      input.model,
      '--output-format',
      'json',
      '--permission-mode',
      input.mode === 'write' ? 'acceptEdits' : 'plan',
      '--allowedTools',
      input.mode === 'write' ? 'Read,Grep,Glob,Edit,Write' : 'Read,Grep,Glob',
    ],
    command: binary,
    cwd: paths.workDir,
    env: MARKED_ENV,
    stdin: prompt,
    timeoutMs: input.time_limit_s * 1000,
  }),
  read: (stdout) => claudeOutput(stdout),
}

/**
 * OpenCode: `opencode run` with the provider/model, JSON events and the
 * prompt as its message. It works in the directory it is started in, and has
 * no time limit of its own, so the outer timer is the limit.
 */
const opencode: ClientLauncher = {
  command: (binary, input, paths, prompt) => ({
    args: ['run', '--model', input.model, '--format', 'json', prompt],
    command: binary,
    cwd: paths.workDir,
    env: MARKED_ENV,
    timeoutMs: input.time_limit_s * 1000,
  }),
  read: (stdout) => openCodeOutput(stdout),
}

/**
 * Hermes, one-shot: the wrapped prompt as its argument, the client model and
 * the provider this machine allows, and the usage file it writes its own
 * tokens and dollars to. Hermes loads the working directory's rules and memory
 * as it always does, so the run carries the mark that stops it delegating on
 * and nothing else. It has no time limit of its own, so the outer timer is the
 * limit.
 */
const hermes: ClientLauncher = {
  command: (binary, input, paths, prompt) => ({
    args: ['-z', prompt, '--usage-file', join(paths.runDir, 'usage.json'), '-m', input.model, '--provider', String(input.provider)],
    command: binary,
    cwd: paths.workDir,
    env: MARKED_ENV,
    timeoutMs: (input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000,
  }),
  read: (stdout, usage) => hermesOutput(stdout, usage),
  reportPath: (paths) => join(paths.runDir, 'usage.json'),
}

/** The launcher of every client a run can be handed to. */
export const LAUNCHERS: Readonly<Record<RunClient, ClientLauncher>> = { agy, claude, hermes, opencode, vibe }
