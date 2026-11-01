import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import { GhCliAdapter } from '../adapters/gh/index.js'
import { NodeProcessAdapter } from '../adapters/process/index.js'
import {
  AgyCriticClient,
  createCritiqueCommands,
  CritiqueError,
  NodeCritiqueFiles,
  VibeCriticClient,
  type CritiqueCommands,
} from '../contexts/critique/index.js'
import { GitCliAdapter } from '../contexts/graph/index.js'

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
  'check-tickets': 'checkTickets',
  'fan-out': 'fanOut',
  prepare: 'prepare',
  record: 'record',
  state: 'state',
  stats: 'stats',
  triage: 'triage',
} as const satisfies Record<string, keyof CritiqueCommands>

export const CRITIQUE_USAGE = `usage: critique <prepare|fan-out|record|allow|check-tickets|stats|triage|state|budget> [--input FILE | --json TEXT]

Each command reads one JSON object (from FILE, from TEXT, or from stdin) and
prints one JSON object. A refusal is a normal result with "status": "refused".
An error prints "status": "error" with its "code" and exits 1. A check that
fails, such as check-tickets finding a dependency cycle, prints
"status": "failed" and exits 3.
`

/** The exit code of a result whose status is `failed`: a check failed, though the command did not. */
export const FAILED_EXIT_CODE = 3

/** The runner's command line: one command, one JSON object in, one JSON object out. */
export async function runCritiqueCli(argv: readonly string[], runtime: CritiqueCliRuntime): Promise<CritiqueCliResult> {
  const [name, flag, value, ...rest] = argv
  if (name === '--help' || name === '-h') return { exitCode: 0, stderr: '', stdout: CRITIQUE_USAGE }
  const method = name !== undefined && Object.hasOwn(COMMANDS, name) ? COMMANDS[name as keyof typeof COMMANDS] : undefined
  const flagOk = flag === undefined || ((flag === '--input' || flag === '--json') && value !== undefined)
  if (method === undefined || !flagOk || rest.length > 0) {
    return { exitCode: 2, stderr: CRITIQUE_USAGE, stdout: '' }
  }
  try {
    const text =
      flag === '--json' ? (value ?? '') : flag === '--input' ? await runtime.readFile(value ?? '') : await runtime.readStdin()
    const result = await runtime.commands[method].run(parseInput(text))
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

function parseInput(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch (error) {
    throw new CritiqueError('invalid-input', `the input is not JSON: ${String(error)}`)
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
  return {
    commands: createCritiqueCommands({
      assetsRoot,
      clients: {
        agy: new AgyCriticClient(processPort, files, env, platform),
        vibe: new VibeCriticClient(processPort, files, env),
      },
      clock: new SystemClockAdapter(),
      env,
      files,
      gh: new GhCliAdapter(processPort),
      git: new GitCliAdapter(processPort),
    }),
    readFile: (path) => files.readText(path),
    readStdin,
  }
}
