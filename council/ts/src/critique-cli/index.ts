import { join } from 'node:path'

import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import { GhCliAdapter } from '../adapters/gh/index.js'
import { NodeProcessAdapter } from '../adapters/process/index.js'
import {
  AgyCriticClient,
  createCritiqueCommands,
  CritiqueError,
  GatewayGrants,
  GatewayPlanLimit,
  NodeCritiqueFiles,
  ProcessSearchClient,
  VibeCriticClient,
  type CritiqueCommands,
} from '../contexts/critique/index.js'
import { GitCliAdapter } from '../contexts/graph/index.js'
import { NodeModelGatewayLedger } from '../contexts/model-gateway/index.js'

export { runSearchServer } from '../contexts/critique/index.js'

export interface CritiqueCliResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

export interface CritiqueCliRuntime {
  readonly commands: CritiqueCommands
  readFile(path: string): Promise<string>
  readStdin(): Promise<string>
}

const COMMANDS = {
  allow: 'allow',
  budget: 'budget',
  'check-adrs': 'checkAdrs',
  'check-plan': 'checkPlan',
  'check-rewrite': 'checkRewrite',
  'check-tickets': 'checkTickets',
  'cold-read': 'coldRead',
  facts: 'facts',
  'fan-out': 'fanOut',
  gate: 'gate',
  prepare: 'prepare',
  record: 'record',
  refute: 'refute',
  reviews: 'reviews',
  'session-start': 'sessionStart',
  setup: 'setup',
  state: 'state',
  stats: 'stats',
  triage: 'triage',
} as const satisfies Record<string, keyof CritiqueCommands>

export const CRITIQUE_USAGE = `usage: critique <prepare|fan-out|refute|record|allow|check-tickets|check-adrs|check-rewrite|cold-read|check-plan|facts|stats|triage|state|budget|reviews|gate|session-start|setup> [--input FILE | --json TEXT]
       critique hook <critique-commit-gate|critique-pr-gate|critique-session-start>
       critique search-server --root DIR [--max-matches N] [--max-bytes N]

Each command reads one JSON object (from FILE, from TEXT, or from stdin) and
prints one JSON object. A refusal is a normal result with "status": "refused".
An error prints "status": "error" with its "code" and exits 1. A check that
fails, such as check-tickets finding a dependency cycle or check-adrs a
structural error in the decision log, check-rewrite a changed decision
sentence or check-plan a rule of the plan format, prints
"status": "failed" and exits 3. gate and session-start always exit 0; input
they cannot read is allowed, and logged.

hook runs one of ADR-0002's hooks on the agent's payload from stdin and
prints only what the agent reads: a gate's deny, the session-start line, or
nothing. It always exits 0.

search-server is the read-only MCP stdio server a Vibe critic searches its
packet copy through (ADR-0006); the runner starts it, never the owner.
`

/** The exit code of a result whose status is `failed`: a check failed, though the command did not. */
export const FAILED_EXIT_CODE = 3

/** The hooks ADR-0002 allows, each the gate or decision it runs. */
const HOOKS = {
  'critique-commit-gate': 'commit',
  'critique-pr-gate': 'pr',
  'critique-session-start': 'session-start',
} as const

/** The runner's command line: one command, one JSON object in, one JSON object out. */
export async function runCritiqueCli(argv: readonly string[], runtime: CritiqueCliRuntime): Promise<CritiqueCliResult> {
  const [name, flag, value, ...rest] = argv
  if (name === '--help' || name === '-h') return { exitCode: 0, stderr: '', stdout: CRITIQUE_USAGE }
  if (name === 'hook') {
    const hook = flag !== undefined && value === undefined && Object.hasOwn(HOOKS, flag) ? HOOKS[flag as keyof typeof HOOKS] : undefined
    // Exit 1, never 2: an agent treats exit 2 from a PreToolUse hook as a block.
    if (hook === undefined) return { exitCode: 1, stderr: CRITIQUE_USAGE, stdout: '' }
    return { exitCode: 0, stderr: '', stdout: await runHook(hook, runtime).catch(() => '') }
  }
  const method = name !== undefined && Object.hasOwn(COMMANDS, name) ? COMMANDS[name as keyof typeof COMMANDS] : undefined
  const flagOk = flag === undefined || ((flag === '--input' || flag === '--json') && value !== undefined)
  if (method === undefined || !flagOk || rest.length > 0) {
    return { exitCode: 2, stderr: CRITIQUE_USAGE, stdout: '' }
  }
  try {
    const text =
      flag === '--json' ? (value ?? '') : flag === '--input' ? await runtime.readFile(value ?? '') : await runtime.readStdin()
    // The gate fails open: input that is not JSON is the gate's to log and allow, not an error.
    const lenient = method === 'gate' || method === 'sessionStart'
    const result = await runtime.commands[method].run(lenient ? parseLeniently(text) : parseInput(text))
    const exitCode = result.status === 'failed' ? FAILED_EXIT_CODE : 0
    return { exitCode, stderr: '', stdout: `${JSON.stringify(result, null, 2)}\n` }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    const code = error instanceof CritiqueError ? error.code : 'unexpected'
    return {
      exitCode: 1,
      stderr: `critique ${name ?? ''}: ${message}\n`,
      stdout: `${JSON.stringify({ code, message, status: 'error' })}\n`,
    }
  }
}

/**
 * What one hook prints for the agent: a gate's `hook_output` as it is, the
 * session-start line as plain text, or nothing (ADR-0002, #197).
 */
async function runHook(hook: (typeof HOOKS)[keyof typeof HOOKS], runtime: CritiqueCliRuntime): Promise<string> {
  const payload = parseLeniently(await runtime.readStdin())
  if (hook === 'session-start') {
    const result = await runtime.commands.sessionStart.run(payload)
    return result.line === null ? '' : `${result.line}\n`
  }
  const result = await runtime.commands.gate.run(isJsonObject(payload) ? { ...payload, gate: hook } : payload)
  return result.status === 'blocked' ? `${JSON.stringify(result.hook_output)}\n` : ''
}

function isJsonObject(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function parseInput(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch (error) {
    throw new CritiqueError('invalid-input', `the input is not JSON: ${String(error)}`)
  }
}

function parseLeniently(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return text
  }
}

/** Wires the real adapters. `assetsRoot` is the directory the bundled runner ships in. */
export function defaultCritiqueRuntime(
  assetsRoot: string,
  readStdin: () => Promise<string>,
  environment: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): CritiqueCliRuntime {
  const files = new NodeCritiqueFiles()
  const processPort = new NodeProcessAdapter()
  const env = new ProcessEnvAdapter(environment)
  const clock = new SystemClockAdapter()
  return {
    commands: createCritiqueCommands({
      assetsRoot,
      clients: {
        agy: new AgyCriticClient(processPort, files, env, platform),
        vibe: new VibeCriticClient(processPort, files, env, platform),
      },
      clock,
      env,
      files,
      gh: new GhCliAdapter(processPort),
      git: new GitCliAdapter(processPort),
      // Every critic run is recorded in the gateway's ledger, and Vibe's plan
      // limit is read the way the gateway reads it, from its shipped defaults
      // beside this bundle.
      // Codex critics of a nedap repository run under the gateway's grants.
      grants: new GatewayGrants(env, clock),
      ledger: new NodeModelGatewayLedger(env),
      plan: new GatewayPlanLimit(env, clock, join(assetsRoot, '..', 'model-gateway', 'defaults.json')),
      // Vibe starts the search server as `node critique.mjs search-server ...`.
      search: new ProcessSearchClient(processPort, { args: [join(assetsRoot, 'critique.mjs')], command: process.execPath }),
    }),
    readFile: (path) => files.readText(path),
    readStdin,
  }
}
