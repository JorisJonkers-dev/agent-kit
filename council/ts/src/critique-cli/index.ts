import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import { NodeProcessAdapter } from '../adapters/process/index.js'
import {
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
  'fan-out': 'fanOut',
  prepare: 'prepare',
  record: 'record',
} as const satisfies Record<string, keyof CritiqueCommands>

export const CRITIQUE_USAGE = `usage: critique <prepare|fan-out|record|allow> [--input FILE | --json TEXT]

Each command reads one JSON object (from FILE, from TEXT, or from stdin) and
prints one JSON object. A refusal is a normal result with "status": "refused".
An error prints "status": "error" with its "code" and exits 1.
`

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
    const result = await runtime.commands[method](parseInput(text))
    return { exitCode: 0, stderr: '', stdout: `${JSON.stringify(result, null, 2)}\n` }
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
): CritiqueCliRuntime {
  const files = new NodeCritiqueFiles()
  const processPort = new NodeProcessAdapter()
  return {
    commands: createCritiqueCommands({
      assetsRoot,
      clients: { vibe: new VibeCriticClient(processPort, files) },
      clock: new SystemClockAdapter(),
      env: new ProcessEnvAdapter(environment),
      files,
      git: new GitCliAdapter(processPort),
    }),
    readFile: (path) => files.readText(path),
    readStdin,
  }
}
