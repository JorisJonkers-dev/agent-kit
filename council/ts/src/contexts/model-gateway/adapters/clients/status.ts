import type { ProcessResult } from '../../../../ports/index.js'
import type { ClientStatusPort, ModelGatewayDependencies, ModelGatewayProcessPort } from '../../application/ports.js'
import type { ClientStatusReading } from '../../domain/usage.js'

/**
 * Each client's own status command, run read-only through the process port:
 * it says whether the client is signed in, and never sends a prompt. A client
 * whose status command cannot be run, or does not answer in time, is
 * `unknown`, not signed out: silence is not a refusal.
 */

/** How long a status command may take before the gateway stops waiting for it. */
export const STATUS_TIMEOUT_MS = 5_000

/** What a status command that outlived its time limit leaves the gateway knowing. */
const TIMED_OUT = 'status command did not answer in time'

/** What running a status command came to: its answer, or why it could not be run. */
type StatusRun = { readonly result: ProcessResult } | { readonly error: string }

/** What a status command's own answer says, once it has answered at all. */
interface StatusVerdict {
  readonly signed_in: 'yes' | 'no' | 'unknown'
  readonly detail: string
}

async function runStatus(
  process: ModelGatewayProcessPort,
  command: string,
  args: readonly string[],
  timeoutMs: number,
): Promise<StatusRun> {
  try {
    return { result: await process.exec({ args: [...args], command, timeoutMs }) }
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * A status command's reading: the two answers it can never give are that it
 * did not run at all, and that it did not answer in time. Both are `unknown`,
 * so a hung or broken client never reads as signed out.
 */
function readingOf(command: string, run: StatusRun, decide: (result: ProcessResult) => StatusVerdict): ClientStatusReading {
  if ('error' in run) return { command, detail: `could not be read: ${run.error}`, signed_in: 'unknown' }
  if (run.result.timedOut === true) return { command, detail: TIMED_OUT, signed_in: 'unknown' }
  return { command, ...decide(run.result) }
}

/** The first line a command printed that says something, without its indentation. */
function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line !== '')
}

/**
 * A login status: `codex login status` prints `Logged in using ChatGPT` and
 * exits 0, and says why it is not logged in on a non-zero exit.
 */
function loginReading(result: ProcessResult): StatusVerdict {
  const detail = firstLine(result.stdout) ?? firstLine(result.stderr) ?? `exited ${String(result.exitCode)}`
  return { detail, signed_in: result.exitCode === 0 ? 'yes' : 'no' }
}

/**
 * A credential listing: `hermes auth list` and `opencode auth list` name the
 * credentials the client holds, so a listing that exits 0 and names none is a
 * client with none. Any other non-zero exit is a command the gateway could not
 * read, which is `unknown`: a client that answers `unknown command` is not
 * signed out.
 */
function listingReading(result: ProcessResult): StatusVerdict {
  if (result.exitCode !== 0) {
    const complaint = firstLine(result.stderr)
    return {
      detail: `could not be read: exited ${String(result.exitCode)}${complaint === undefined ? '' : `: ${complaint}`}`,
      signed_in: 'unknown',
    }
  }
  const listed = result.stdout.trim() !== ''
  return { detail: listed ? 'it lists credentials' : 'it lists no credentials', signed_in: listed ? 'yes' : 'no' }
}

/** Codex's own login status. */
export class CodexStatus implements ClientStatusPort {
  constructor(
    private readonly process: ModelGatewayProcessPort,
    private readonly timeoutMs: number = STATUS_TIMEOUT_MS,
  ) {}

  async read(): Promise<ClientStatusReading> {
    const run = await runStatus(this.process, 'codex', ['login', 'status'], this.timeoutMs)
    return readingOf('codex login status', run, loginReading)
  }
}

/** Hermes's own credential listing, which needs no provider named. */
export class HermesStatus implements ClientStatusPort {
  constructor(
    private readonly process: ModelGatewayProcessPort,
    private readonly timeoutMs: number = STATUS_TIMEOUT_MS,
  ) {}

  async read(): Promise<ClientStatusReading> {
    const run = await runStatus(this.process, 'hermes', ['auth', 'list'], this.timeoutMs)
    return readingOf('hermes auth list', run, listingReading)
  }
}

/** OpenCode's own credential listing. */
export class OpenCodeStatus implements ClientStatusPort {
  constructor(
    private readonly process: ModelGatewayProcessPort,
    private readonly timeoutMs: number = STATUS_TIMEOUT_MS,
  ) {}

  async read(): Promise<ClientStatusReading> {
    const run = await runStatus(this.process, 'opencode', ['auth', 'list'], this.timeoutMs)
    return readingOf('opencode auth list', run, listingReading)
  }
}

/**
 * The clients whose own status command says whether they are signed in. Vibe,
 * Antigravity and Claude Code have none the gateway can read, so they are left
 * out and report `signed_in: "unknown"`.
 */
export function clientStatuses(
  deps: Pick<ModelGatewayDependencies, 'process'>,
): Readonly<Record<string, ClientStatusPort>> {
  return {
    codex: new CodexStatus(deps.process),
    hermes: new HermesStatus(deps.process),
    opencode: new OpenCodeStatus(deps.process),
  }
}
